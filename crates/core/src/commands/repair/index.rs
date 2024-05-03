//! `repair` index subcommand
use derive_setters::Setters;
use log::{debug, info, warn};

use std::collections::HashMap;

use crate::{
    backend::{
        decrypt::{DecryptReadBackend, DecryptWriteBackend},
        FileType, ReadBackend, WriteBackend,
    },
    error::{CommandErrorKind, RusticErrorKind, RusticResult},
    index::{binarysorted::IndexCollector, indexer::Indexer, GlobalIndex},
    progress::{Progress, ProgressBars},
    repofile::{IndexFile, IndexPack, PackHeader, PackHeaderRef},
    repository::{Open, Repository},
    Id,
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

impl RepairIndexOptions {
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
    pub(crate) fn repair<P: ProgressBars, S: Open>(
        self,
        repo: &Repository<P, S>,
        dry_run: bool,
    ) -> RusticResult<()> {
        if repo.config().append_only == Some(true) {
            return Err(
                CommandErrorKind::NotAllowedWithAppendOnly("index repair".to_string()).into(),
            );
        }

        let be = repo.dbe();
        let mut checker = PackChecker::new(repo)?;

        let p = repo.pb.progress_counter("reading index...");
        for index in be.stream_all::<IndexFile>(&p)? {
            let (index_id, index) = index?;
            let (new_index, changed) = checker.check_pack(index, self.read_all);
            match (changed, dry_run) {
                (true, true) => info!("would have modified index file {index_id}"),
                (true, false) => {
                    if !new_index.packs.is_empty() || !new_index.packs_to_delete.is_empty() {
                        _ = be.save_file(&new_index)?;
                    }
                    be.remove(FileType::Index, &index_id, true)
                        .map_err(RusticErrorKind::Backend)?;
                }
                (false, _) => {} // nothing to do
            }
        }
        p.finish();

        let pack_read_header = checker.into_pack_to_read();
        repo.warm_up_wait(pack_read_header.iter().map(|(id, _, _)| *id))?;

        let indexer = Indexer::new(be.clone()).into_shared();
        let p = repo.pb.progress_counter("reading pack headers");
        p.set_length(
            pack_read_header
                .len()
                .try_into()
                .map_err(CommandErrorKind::ConversionFromIntFailed)?,
        );
        for (id, size_hint, packsize) in pack_read_header {
            debug!("reading pack {id}...");
            match PackHeader::from_file(be, id, size_hint, packsize) {
                Err(err) => {
                    warn!("error reading pack {id} (-> removing from index): {err}");
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
}

struct PackChecker {
    packs: HashMap<Id, u32>,
    packs_to_read: Vec<(Id, Option<u32>, u32)>,
}

impl PackChecker {
    fn new<P: ProgressBars, S: Open>(repo: &Repository<P, S>) -> RusticResult<Self> {
        let be = repo.dbe();
        let p = repo.pb.progress_spinner("listing packs...");
        let packs: HashMap<_, _> = be
            .list_with_size(FileType::Pack)
            .map_err(RusticErrorKind::Backend)?
            .into_iter()
            .collect();
        p.finish();

        Ok(Self {
            packs,
            packs_to_read: Vec::new(),
        })
    }

    fn check_pack(&mut self, indexfile: IndexFile, read_all: bool) -> (IndexFile, bool) {
        let mut new_index = IndexFile::default();
        let mut changed = false;
        for (p, to_delete) in indexfile.all_packs() {
            let index_size = p.pack_size();
            let id = p.id;
            match self.packs.remove(&id) {
                None => {
                    // this pack either does not exist or was already indexed in another index file => remove from index!
                    debug!("removing non-existing pack {id} from index");
                    changed = true;
                }
                Some(size) => {
                    if index_size != size {
                        info!("pack {id}: size computed by index: {index_size}, actual size: {size}, will re-read header");
                    }

                    if index_size != size || read_all {
                        // pack exists, but sizes do not match or we want to read all pack files
                        self.packs_to_read.push((
                            id,
                            Some(PackHeaderRef::from_index_pack(&p).size()),
                            size,
                        ));
                    } else {
                        new_index.add(p, to_delete);
                    }
                }
            }
        }
        (new_index, changed)
    }

    fn into_pack_to_read(mut self) -> Vec<(Id, Option<u32>, u32)> {
        // add packs which are listed but not contained in the index
        self.packs_to_read
            .extend(self.packs.into_iter().map(|(id, size)| (id, None, size)));
        self.packs_to_read
    }
}

pub(crate) fn index_checked_from_collector<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
    mut collector: IndexCollector,
) -> RusticResult<GlobalIndex> {
    let mut checker = PackChecker::new(repo)?;
    let be = repo.dbe();

    let p = repo.pb.progress_counter("reading index...");
    for index in be.stream_all::<IndexFile>(&p)? {
        collector.extend(checker.check_pack(index?.1, false).0.packs);
    }
    p.finish();

    let pack_read_header = checker.into_pack_to_read();
    repo.warm_up_wait(pack_read_header.iter().map(|(id, _, _)| *id))?;

    let p = repo.pb.progress_counter("reading pack headers");
    p.set_length(
        pack_read_header
            .len()
            .try_into()
            .map_err(CommandErrorKind::ConversionFromIntFailed)?,
    );
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
