//! `lock` subcommand

use std::collections::BTreeSet;

use chrono::{DateTime, Local};
use derive_setters::Setters;
use log::error;
use rayon::ThreadPoolBuilder;

use crate::{
    backend::{
        decrypt::{DecryptReadBackend, DecryptWriteBackend},
        node::NodeType,
    },
    blob::{tree::TreeStreamerOnce, BlobType},
    error::{CommandErrorKind, RepositoryErrorKind, RusticResult},
    index::{
        binarysorted::{IndexCollector, IndexType},
        indexer::Indexer,
        GlobalIndex, ReadGlobalIndex,
    },
    progress::{Progress, ProgressBars},
    repofile::{
        configfile::ConfigId, IndexFile, IndexId, KeyId, PackId, RepoId, SnapshotFile, SnapshotId,
    },
    repository::Repository,
    BlobId, Open, TreeId, WriteBackend,
};

pub(super) mod constants {
    /// The maximum number of reader threads to use for locking.
    pub(super) const MAX_LOCKER_THREADS_NUM: usize = 20;
}

#[derive(Debug, Clone, Default, Copy, Setters)]
/// Options for the `lock` command
pub struct LockOptions {
    /// Extend locks even if the files are already locked long enough
    always_extend_lock: bool,

    /// Specify until when to extend the lock. If None, lock forever
    until: Option<DateTime<Local>>,
}

impl LockOptions {
    /// Lock the given snapshots and corresponding pack files
    ///
    /// # Errors
    /// TODO
    pub fn lock<P: ProgressBars, S: Open>(
        &self,
        repo: &Repository<P, S>,
        snapshots: &[SnapshotFile],
        now: DateTime<Local>,
    ) -> RusticResult<()> {
        if !repo.be.can_lock() {
            return Err(CommandErrorKind::NoLockingConfigured.into());
        }

        let pb = &repo.pb;
        let be = repo.dbe();

        let mut index_files = Vec::new();

        let p = pb.progress_counter("reading index...");
        let mut index_collector = IndexCollector::new(IndexType::Full);
        for index in be.stream_all::<IndexFile>(&p)? {
            let (id, index) = index?;
            index_collector.extend(index.packs.clone());
            index_files.push((id, index));
        }
        let index = GlobalIndex::new_from_index(index_collector.into_index());
        p.finish();

        let snap_tress = snapshots.iter().map(|sn| sn.tree).collect();
        let packs = find_needed_packs(be, &index, snap_tress, pb)?;
        self.lock_packs(repo, index_files, &packs)?;

        self.lock_snapshots(repo, snapshots, now)?;

        Ok(())
    }

    fn lock_snapshots<P: ProgressBars, S: Open>(
        &self,
        repo: &Repository<P, S>,
        snapshots: &[SnapshotFile],
        now: DateTime<Local>,
    ) -> RusticResult<()> {
        if !repo.be.can_lock() {
            return Err(CommandErrorKind::NoLockingConfigured.into());
        }

        let mut new_snaps = Vec::new();
        let mut remove_snaps = Vec::new();
        let mut lock_snaps = Vec::new();

        for snap in snapshots {
            if !snap.delete.is_locked(self.until) {
                new_snaps.push(SnapshotFile {
                    delete: self.until.into(),
                    ..snap.clone()
                });
                if !snap.must_keep(now) {
                    remove_snaps.push(snap.id);
                }
            } else if self.always_extend_lock {
                lock_snaps.push(snap.id);
            }
        }

        // save new snapshots
        let new_ids = repo.save_snapshots(new_snaps)?;
        lock_snaps.extend(new_ids);

        // remove old snapshots
        repo.delete_snapshots(&remove_snaps)?;

        // Do the actual locking
        lock_files(repo, &lock_snaps, self.until)?;

        Ok(())
    }

    fn lock_packs<P: ProgressBars, S: Open>(
        &self,
        repo: &Repository<P, S>,
        index_files: Vec<(IndexId, IndexFile)>,
        packs: &BTreeSet<PackId>,
    ) -> RusticResult<()> {
        if !repo.be.can_lock() {
            return Err(CommandErrorKind::NoLockingConfigured.into());
        }
        let mut lock_packs = Vec::new();
        let mut remove_index = Vec::new();

        // Check for indexfiles-to-modify and for packs to lock
        // Also already write the new index from the index files which are modified.
        let p = repo.pb.progress_counter("processing index files...");
        p.set_length(index_files.len().try_into().unwrap());
        let indexer = Indexer::new_unindexed(repo.dbe().clone()).into_shared();
        for (id, mut index) in index_files {
            let mut modified = false;
            for pack in &mut index.packs {
                if !packs.contains(&pack.id) {
                    continue;
                }
                if !pack.lock.is_locked(self.until) {
                    pack.lock = self.until.into();
                    modified = true;
                    lock_packs.push(pack.id);
                } else if self.always_extend_lock {
                    lock_packs.push(pack.id);
                }
            }
            if modified {
                for pack in index.packs {
                    indexer.write().unwrap().add(pack)?;
                }
                for pack_remove in index.packs_to_delete {
                    indexer.write().unwrap().add_remove(pack_remove)?;
                }
                remove_index.push(id);
            }
            p.inc(1);
        }
        indexer.write().unwrap().finalize()?;
        p.finish();

        // Remove old index files
        let p = repo.pb.progress_counter("removing old index files...");
        repo.dbe().delete_list(true, remove_index.iter(), p)?;

        // Do the actual locking
        lock_files(repo, &lock_packs, self.until)?;

        Ok(())
    }
}

pub fn lock_repo<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    lock_all_files::<P, S, ConfigId>(repo, until)?;
    lock_all_files::<P, S, KeyId>(repo, until)?;
    lock_all_files::<P, S, SnapshotId>(repo, until)?;
    lock_all_files::<P, S, IndexId>(repo, until)?;
    lock_all_files::<P, S, PackId>(repo, until)?;
    Ok(())
}

pub fn lock_all_files<P: ProgressBars, S, ID: RepoId>(
    repo: &Repository<P, S>,
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    if !repo.be.can_lock() {
        return Err(CommandErrorKind::NoLockingConfigured.into());
    }

    let p = &repo
        .pb
        .progress_spinner(format!("listing {:?} files..", ID::TYPE));
    let ids: Vec<ID> = repo.list()?.collect();
    p.finish();
    lock_files(repo, &ids, until)
}

fn lock_files<P: ProgressBars, S, ID: RepoId>(
    repo: &Repository<P, S>,
    ids: &[ID],
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(constants::MAX_LOCKER_THREADS_NUM)
        .build()
        .map_err(RepositoryErrorKind::FromThreadPoolbilderError)?;
    let p = &repo
        .pb
        .progress_counter(format!("locking {:?} files..", ID::TYPE));
    p.set_length(ids.len().try_into().unwrap());
    let backend = &repo.be;
    pool.in_place_scope(|scope| {
        for id in ids {
            scope.spawn(move |_| {
                if let Err(e) = backend.lock(ID::TYPE, id, until) {
                    // FIXME: Use error handling
                    error!("lock failed for {:?} {id:?}. {e}", ID::TYPE);
                };
                p.inc(1);
            });
        }
    });
    p.finish();
    Ok(())
}

/// Find packs which are needed for the given Trees
///
/// # Arguments
///
/// * `index` - The index to use
/// * `trees` - The trees to consider
/// * `pb` - The progress bars
///
/// # Errors
///
// TODO!: add errors!
fn find_needed_packs(
    be: &impl DecryptReadBackend,
    index: &impl ReadGlobalIndex,
    trees: Vec<TreeId>,
    pb: &impl ProgressBars,
) -> RusticResult<BTreeSet<PackId>> {
    let p = pb.progress_counter("finding needed packs...");

    let mut packs = BTreeSet::new();

    for tree_id in &trees {
        let blob_id = BlobId::from(*tree_id);
        _ = packs.insert(
            index
                .get_id(BlobType::Tree, &blob_id)
                .ok_or_else(|| CommandErrorKind::BlobIdNotFoundinIndex(blob_id))?
                .pack,
        );
    }

    let mut tree_streamer = TreeStreamerOnce::new(be, index, trees, p)?;
    while let Some(item) = tree_streamer.next().transpose()? {
        let (_, tree) = item;
        for node in tree.nodes {
            match node.node_type {
                NodeType::File => {
                    for id in node.content.iter().flatten() {
                        let blob_id = BlobId::from(*id);
                        _ = packs.insert(
                            index
                                .get_id(BlobType::Data, &blob_id)
                                .ok_or_else(|| CommandErrorKind::BlobIdNotFoundinIndex(blob_id))?
                                .pack,
                        );
                    }
                }
                NodeType::Dir => {
                    let id = &node.subtree.unwrap();
                    let blob_id = BlobId::from(*id);
                    _ = packs.insert(
                        index
                            .get_id(BlobType::Tree, &blob_id)
                            .ok_or_else(|| CommandErrorKind::BlobIdNotFoundinIndex(blob_id))?
                            .pack,
                    );
                }
                _ => {} // nothing to do
            }
        }
    }

    Ok(packs)
}
