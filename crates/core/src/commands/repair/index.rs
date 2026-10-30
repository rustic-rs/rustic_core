//! `repair` index subcommand
use derive_setters::Setters;
use log::{debug, info, warn};

use std::collections::HashMap;

use crate::{
    backend::{
        FileType, ReadBackend, WriteBackend,
        decrypt::{DecryptReadBackend, DecryptWriteBackend},
    },
    error::{ErrorKind, RusticError, RusticResult},
    index::{GlobalIndex, binarysorted::IndexCollector, indexer::Indexer},
    repofile::{IndexFile, IndexId, IndexPack, PackHeader, PackHeaderRef, packfile::PackId},
    repository::{Open, Repository},
};

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Default, Debug, Clone, Copy, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `repair index` command
pub struct RepairIndexOptions {
    /// Read all data packs, i.e. completely re-create the index
    #[cfg_attr(feature = "clap", clap(long))]
    pub read_all: bool,
}

/// Runs the `repair index` command
///
/// # Type Parameters
///
/// * `P` - The progress bar type
/// * `S` - The state the repository is in
///
/// # Arguments
///
/// * `repo` - The repository to repair
/// * `dry_run` - Whether to actually modify the repository or just print what would be done
pub(crate) fn repair_index<S: Open>(
    repo: &Repository<S>,
    opts: RepairIndexOptions,
    dry_run: bool,
) -> RusticResult<()> {
    if repo.config().append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Repairing the index is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }

    let be = repo.dbe();
    let mut checker = PackChecker::new(repo)?;

    let p = repo.progress_counter("reading index...");
    for index in be.stream_list::<IndexFile>(std::mem::take(&mut checker.index_ids), &p)? {
        let (index_id, index) = index?;
        let (new_index, changed) = checker.check_pack(index, opts.read_all);
        match (changed, dry_run) {
            (true, true) => info!("would have modified index file {index_id}"),
            (true, false) => {
                if !new_index.packs.is_empty() || !new_index.packs_to_delete.is_empty() {
                    _ = be.save_file(&new_index)?;
                }
                be.remove(FileType::Index, &index_id, true)?;
            }
            (false, _) => {} // nothing to do
        }
    }
    p.finish();

    let pack_read_header = checker.into_pack_to_read();
    repo.warm_up_wait(pack_read_header.iter().map(|(id, _, _)| *id))?;

    let indexer = Indexer::new(be.clone()).into_shared();
    let p = repo.progress_counter("reading pack headers");

    p.set_length(pack_read_header.len().try_into().map_err(|err| {
        RusticError::with_source(
            ErrorKind::Internal,
            "Failed to convert `pack_read_header` length `{length}` to u64.",
            err,
        )
        .attach_context("length", pack_read_header.len().to_string())
    })?);
    for (id, size_hint, packsize) in pack_read_header {
        debug!("reading pack {id}...");
        match PackHeader::from_file(be, id, size_hint, packsize) {
            Err(err) => {
                warn!(
                    "error reading pack {id} (-> removing from index): {}",
                    err.display_log()
                );
            }
            Ok(header) => {
                let pack = IndexPack {
                    blobs: header.into_blobs(),
                    id,
                    ..Default::default()
                };
                if !dry_run {
                    // write pack file to index - without the delete mark
                    indexer.write().unwrap().add_with(pack, false)?;
                }
            }
        }
        p.inc(1);
    }
    indexer.write().unwrap().finalize()?;
    p.finish();

    Ok(())
}

struct PackChecker {
    packs: HashMap<PackId, (u32, bool)>,
    packs_to_read: Vec<(PackId, Option<u32>, u32)>,
    index_ids: Vec<IndexId>,
}

impl PackChecker {
    fn new<S: Open>(repo: &Repository<S>) -> RusticResult<Self> {
        let be = repo.dbe();
        let index_ids: Vec<_> = be
            .list(FileType::Index)?
            .into_iter()
            .map(IndexId::from)
            .collect();
        let p = repo.progress_spinner("listing packs...");
        let mut packs: HashMap<_, _> = be
            .list_with_size(FileType::Pack)?
            .into_iter()
            .map(|(id, size)| (PackId::from(id), (size, false)))
            .collect();
        p.finish();

        let p = repo.progress_counter("reading active packs...");
        for index in be.stream_list::<IndexFile>(index_ids.clone(), &p)? {
            let (_, index) = index?;
            for pack in index.packs {
                if let Some((_, active)) = packs.get_mut(&pack.id) {
                    *active = true;
                }
            }
        }
        p.finish();

        Ok(Self {
            packs,
            packs_to_read: Vec::new(),
            index_ids,
        })
    }

    fn check_pack(&mut self, indexfile: IndexFile, read_all: bool) -> (IndexFile, bool) {
        let mut new_index = IndexFile::default();
        let mut changed = false;
        for (p, to_delete) in indexfile.all_packs() {
            let index_size = p.pack_size();
            let id = p.id;
            if to_delete && self.packs.get(&id).is_some_and(|(_, active)| *active) {
                changed = true;
                continue;
            }
            match self.packs.remove(&id) {
                None => {
                    // this pack either does not exist or was already indexed in another index file => remove from index!
                    debug!("removing non-existing pack {id} from index");
                    changed = true;
                }
                Some((size, _)) => {
                    if index_size != size {
                        info!(
                            "pack {id}: size computed by index: {index_size}, actual size: {size}, will re-read header"
                        );
                    }

                    if index_size != size || read_all {
                        // pack exists, but sizes do not match or we want to read all pack files
                        self.packs_to_read.push((
                            id,
                            Some(PackHeaderRef::from_index_pack(&p).size()),
                            size,
                        ));
                        changed = true;
                    } else {
                        new_index.add(p, to_delete);
                    }
                }
            }
        }
        (new_index, changed)
    }

    fn into_pack_to_read(mut self) -> Vec<(PackId, Option<u32>, u32)> {
        // add packs which are listed but not contained in the index
        self.packs_to_read
            .extend(self.packs.into_iter().map(|(id, (size, _))| (id, None, size)));
        self.packs_to_read
    }
}

pub(crate) fn index_checked_from_collector<S: Open>(
    repo: &Repository<S>,
    mut collector: IndexCollector,
) -> RusticResult<GlobalIndex> {
    let mut checker = PackChecker::new(repo)?;
    let be = repo.dbe();

    let p = repo.progress_counter("reading index...");
    for index in be.stream_list::<IndexFile>(std::mem::take(&mut checker.index_ids), &p)? {
        collector.extend(checker.check_pack(index?.1, false).0.packs);
    }
    p.finish();

    let pack_read_header = checker.into_pack_to_read();
    repo.warm_up_wait(pack_read_header.iter().map(|(id, _, _)| *id))?;

    let p = repo.progress_counter("reading pack headers");

    p.set_length(pack_read_header.len().try_into().map_err(|err| {
        RusticError::with_source(
            ErrorKind::Internal,
            "Failed to convert `pack_read_header` length `{length}`  to u64.",
            err,
        )
        .attach_context("length", pack_read_header.len().to_string())
    })?);

    let index_packs: Vec<_> = pack_read_header
        .into_iter()
        .map(|(id, size_hint, packsize)| {
            debug!("reading pack {id}...");
            let pack = IndexPack {
                id,
                blobs: PackHeader::from_file(be, id, size_hint, packsize)?.into_blobs(),
                ..Default::default()
            };
            p.inc(1);
            Ok(pack)
        })
        .collect::<RusticResult<_>>()?;
    p.finish();

    collector.extend(index_packs);
    Ok(GlobalIndex::new_from_index(collector.into_index()))
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use rstest::rstest;

    use crate::{
        BackupOptions, ConfigOptions, Credentials, FileType, IndexedIdsStatus, KeyOptions,
        ParentOptions, ReadBackend, RepairIndexOptions, Repository, RepositoryBackends,
        RepositoryOptions, WriteBackend,
        backend::{MockBackend, decrypt::DecryptWriteBackend},
        blob::{BlobId, tree::Tree},
        index::{
            GlobalIndex, ReadIndex,
            binarysorted::{IndexCollector, IndexType},
        },
        repofile::{IndexFile, MasterKey, SnapshotFile},
        test_support::{MemoryBackend, MemorySource},
    };

    use super::PackChecker;

    #[test]
    fn repair_preserves_index_published_after_pack_listing() -> anyhow::Result<()> {
        let (seed, snapshot) = repository_with_snapshot()?;
        let inner = seed.be.clone();
        let published = Arc::new(AtomicBool::new(false));
        let mut backend = MockBackend::new();
        _ = backend.expect_location().return_const("memory".to_string());
        let be = inner.clone();
        let publication = published.clone();
        _ = backend.expect_list_with_size().returning(move |tpe| {
            if tpe == FileType::Index && !publication.load(Ordering::SeqCst) {
                return Ok(Vec::new());
            }
            if tpe == FileType::Pack && !publication.swap(true, Ordering::SeqCst) {
                return Ok(Vec::new());
            }
            be.list_with_size(tpe)
        });
        let be = inner.clone();
        _ = backend
            .expect_read_full()
            .returning(move |tpe, id| be.read_full(tpe, id));
        let be = inner.clone();
        _ = backend
            .expect_read_partial()
            .returning(move |tpe, id, cacheable, offset, length| {
                be.read_partial(tpe, id, cacheable, offset, length)
            });
        let be = inner.clone();
        _ = backend
            .expect_write_bytes()
            .returning(move |tpe, id, cacheable, content| {
                be.write_bytes(tpe, id, cacheable, content)
            });
        _ = backend
            .expect_remove()
            .returning(move |tpe, id, cacheable| inner.remove(tpe, id, cacheable));

        let backends = RepositoryBackends::new(Arc::new(backend), None);
        let repo = Repository::new(&RepositoryOptions::default().no_cache(true), &backends)?
            .open(&Credentials::Masterkey(seed.key()))?;
        repo.repair_index(&RepairIndexOptions::default(), false)?;
        assert!(published.load(Ordering::SeqCst));
        let repo = repo.to_indexed()?;
        assert!(
            repo.index().has_tree(&snapshot.tree),
            "repair must leave a newly published index intact",
        );
        assert!(!repo.get_tree(&snapshot.tree)?.nodes.is_empty());
        Ok(())
    }

    fn repository_with_snapshot() -> anyhow::Result<(Repository<IndexedIdsStatus>, SnapshotFile)> {
        let backends = RepositoryBackends::new(Arc::new(MemoryBackend::default()), None);
        let options = RepositoryOptions::default().no_cache(true);
        let credentials = Credentials::Masterkey(MasterKey::new());
        let repo = Repository::new(&options, &backends)?
            .init(
                &credentials,
                &KeyOptions::default(),
                &ConfigOptions::default(),
            )?
            .to_indexed_ids()?;
        let source = MemorySource {
            files: vec![("queue/00/message", b"message")],
        };
        let snapshot = repo.archive(
            &BackupOptions::default().parent_opts(ParentOptions::default().force(true)),
            &source,
            SnapshotFile::default(),
            &[PathBuf::from("source")],
        )?;
        Ok((repo, snapshot))
    }

    #[rstest]
    #[case::marked_before_active(true)]
    #[case::active_before_marked(false)]
    fn repair_preserves_active_pack_regardless_of_index_order(
        #[case] marked_first: bool,
    ) -> anyhow::Result<()> {
        let (repo, snapshot) = repository_with_snapshot()?;
        let mut indexes = repo
            .stream_files::<IndexFile>()?
            .collect::<crate::RusticResult<Vec<_>>>()?
            .into_iter()
            .map(|(_, index)| index)
            .collect::<Vec<_>>();
        let root = BlobId::from(snapshot.tree);
        let pack = indexes
            .iter()
            .flat_map(|index| &index.packs)
            .find(|pack| pack.blobs.iter().any(|blob| blob.id == root))
            .unwrap()
            .clone();
        let pack_id = pack.id;
        let marked = IndexFile {
            packs_to_delete: vec![pack],
            ..Default::default()
        };
        _ = repo.dbe().save_file(&marked)?;
        if marked_first {
            indexes.insert(0, marked);
        } else {
            indexes.push(marked);
        }

        let mut checker = PackChecker::new(&repo)?;
        let repaired = indexes
            .into_iter()
            .map(|index| checker.check_pack(index, false).0)
            .collect::<Vec<_>>();
        assert!(checker.into_pack_to_read().is_empty());
        assert_eq!(
            repaired
                .iter()
                .flat_map(|index| &index.packs)
                .filter(|pack| pack.id == pack_id)
                .count(),
            1,
            "repair must preserve the active entry even when its deletion mark is processed first",
        );
        assert!(
            repaired
                .iter()
                .flat_map(|index| &index.packs_to_delete)
                .all(|pack| pack.id != pack_id),
            "a stale deletion mark must not shadow an active pack",
        );

        let mut collector = IndexCollector::new(IndexType::Full);
        collector.extend(repaired.into_iter().flat_map(|index| index.packs));
        let index = GlobalIndex::new_from_index(collector.into_index());
        assert!(
            !Tree::from_backend(repo.dbe(), &index, snapshot.tree)?
                .nodes
                .is_empty()
        );
        repo.repair_index(&RepairIndexOptions::default(), false)?;
        let repo = repo.to_indexed()?;
        assert!(repo.index().has_tree(&snapshot.tree));
        assert!(repo.stream_files::<IndexFile>()?.all(|item| {
            item.unwrap()
                .1
                .packs_to_delete
                .iter()
                .all(|p| p.id != pack_id)
        }));
        Ok(())
    }
}
