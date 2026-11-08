//! `prune` subcommand

/// App-local prelude includes `app_reader()`/`app_writer()`/`app_config()`
/// accessors along with logging macros. Customize as you see fit.
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use bytesize::ByteSize;
use derive_more::Add;
use derive_setters::Setters;
use enumset::{EnumSet, EnumSetType};
use itertools::Itertools;
use jiff::{Span, Timestamp, Zoned};
use log::info;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};

use crate::{
    backend::{
        FileType, ReadBackend,
        decrypt::{DecryptReadBackend, DecryptWriteBackend},
        node::{Node, NodeType},
    },
    blob::{
        BlobId, BlobLocations, BlobType, BlobTypeMap, Initialize,
        packer::{BlobCopier, CopyPackBlobs, PackSizer},
        tree::TreeStreamerOnce,
    },
    error::{ErrorKind, RusticError, RusticResult},
    index::{
        GlobalIndex, ReadGlobalIndex, ReadIndex,
        binarysorted::{IndexCollector, IndexType},
        indexer::Indexer,
    },
    repofile::{
        HeaderEntry, IndexBlob, IndexFile, IndexPack, SnapshotFile, SnapshotId,
        indexfile::{IndexId, IndexMarkType},
        packfile::PackId,
    },
    repository::{Open, Repository},
};

pub(super) mod constants {
    /// Minimum size of an index file to be considered for pruning
    pub(super) const MIN_INDEX_LEN: usize = 10_000;
}

#[allow(clippy::struct_excessive_bools)]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Debug, Clone, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `prune` command
pub struct PruneOptions {
    /// Define maximum data to repack in % of reposize or as size (e.g. '5b', '2 kB', '3M', '4TiB') or 'unlimited'
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "LIMIT", default_value = "10%")
    )]
    pub max_repack: LimitOption,

    /// Tolerate limit of unused data in % of reposize after pruning or as size (e.g. '5b', '2 kB', '3M', '4TiB') or 'unlimited'
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "LIMIT", default_value = "5%")
    )]
    pub max_unused: LimitOption,

    /// Minimum duration (e.g. 90d) to keep packs before repacking or removing. More recently created
    /// packs won't be repacked or marked for deletion within this prune run.
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "DURATION", default_value = "0d")
    )]
    pub keep_pack: Span,

    /// Minimum duration (e.g. 10m) to keep packs marked. More recently marked packs won't be
    /// deleted or recovered within this prune run.
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "DURATION", default_value = "23h")
    )]
    pub keep_marked: Span,

    /// Delete files immediately instead of marking them and immediately recover instead of marking for recovery. Also applies for already marked pack files.
    ///
    /// # Warning
    ///
    /// * Only use if you are sure the repository is not accessed by parallel processes!
    #[cfg_attr(feature = "clap", clap(long))]
    pub instant_delete: bool,

    /// Delete index files early if instant-delete is chosen. This allows to run prune if there is few or no space left.
    ///
    /// # Warning
    ///
    /// * If prune aborts, this can lead to a (partly) missing index which must be repaired!
    #[cfg_attr(feature = "clap", clap(long))]
    pub early_delete_index: bool,

    /// Simply copy blobs when repacking instead of decrypting; possibly compressing; encrypting
    #[cfg_attr(feature = "clap", clap(long))]
    pub fast_repack: bool,

    /// Repack packs containing uncompressed blobs. This cannot be used with --fast-repack.
    /// Implies --max-unused=0.
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "fast_repack"))]
    pub repack_uncompressed: bool,

    /// Repack all packs. Implies --max-unused=0.
    #[cfg_attr(feature = "clap", clap(long))]
    pub repack_all: bool,

    /// Only repack packs which are cacheable [default: true for a hot/cold repository, else false]
    #[cfg_attr(feature = "clap", clap(long, value_name = "TRUE/FALSE"))]
    pub repack_cacheable_only: Option<bool>,

    /// Do not repack packs which only needs to be resized
    #[cfg_attr(feature = "clap", clap(long))]
    pub no_resize: bool,

    #[cfg_attr(feature = "clap", clap(skip))]
    /// Ignore these snapshots when looking for data-still-in-use.
    ///
    /// # Warning
    ///
    /// * Use this option with care!
    /// * If you specify snapshots which are not deleted, running the resulting `PrunePlan`
    ///   will remove data which is used within those snapshots!
    pub ignore_snaps: Vec<SnapshotId>,
}

impl Default for PruneOptions {
    fn default() -> Self {
        Self {
            max_repack: LimitOption::Percentage(10),
            max_unused: LimitOption::Percentage(5),
            keep_pack: Span::new(),
            keep_marked: Span::new().hours(23),
            instant_delete: false,
            early_delete_index: false,
            fast_repack: false,
            repack_uncompressed: false,
            repack_all: false,
            repack_cacheable_only: None,
            no_resize: false,
            ignore_snaps: Vec::new(),
        }
    }
}

/// Enum to specify a size limit
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum LimitOption {
    /// Size in bytes
    Size(ByteSize),
    /// Size in percentage of repository size
    Percentage(u64),
    /// No limit
    Unlimited,
}

impl FromStr for LimitOption {
    type Err = Box<RusticError>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.chars().last().unwrap_or('0') {
            '%' => Self::Percentage({
                let mut copy = s.to_string();
                _ = copy.pop();
                copy.parse().map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Failed to parse percentage limit `{limit}`",
                        err,
                    )
                    .attach_context("limit", s)
                })?
            }),
            'd' if s == "unlimited" => Self::Unlimited,
            _ => {
                let byte_size = ByteSize::from_str(s).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Failed to parse size limit `{limit}`",
                        err,
                    )
                    .attach_context("limit", s)
                })?;

                Self::Size(byte_size)
            }
        })
    }
}

#[derive(EnumSetType, Debug, PartialOrd, Ord, Serialize, Deserialize)]
#[enumset(serialize_repr = "list")]
pub enum PackStatus {
    NotCompressed,
    TooYoung,
    TimeNotSet,
    TooLarge,
    TooSmall,
    HasUnusedBlobs,
    HasUsedBlobs,
    Marked,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DebugDetailedStats {
    pub packs: u64,
    pub unused_blobs: u64,
    pub unused_size: u64,
    pub used_blobs: u64,
    pub used_size: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct DebugStatsKey {
    pub todo: PackToDo,
    pub blob_type: BlobType,
    pub status: EnumSet<PackStatus>,
}

#[derive(Debug, Default, Serialize)]
pub struct DebugStats(pub BTreeMap<DebugStatsKey, DebugDetailedStats>);

impl DebugStats {
    fn add(&mut self, pi: &PackInfo, todo: PackToDo, status: EnumSet<PackStatus>) {
        let blob_type = pi.blob_type;
        let details = self
            .0
            .entry(DebugStatsKey {
                todo,
                blob_type,
                status,
            })
            .or_insert(DebugDetailedStats {
                packs: 0,
                unused_blobs: 0,
                unused_size: 0,
                used_blobs: 0,
                used_size: 0,
            });
        details.packs += 1;
        details.unused_blobs += u64::from(pi.unused_blobs);
        details.unused_size += u64::from(pi.unused_size);
        details.used_blobs += u64::from(pi.used_blobs);
        details.used_size += u64::from(pi.used_size);
    }
}

/// Statistics about what is deleted or kept within `prune`
#[derive(Default, Debug, Clone, Copy)]
pub struct DeleteStats {
    /// Number of blobs to remove
    pub remove: u64,
    /// Number of blobs to recover
    pub recover: u64,
    /// Number of blobs to keep
    pub keep: u64,
}

impl DeleteStats {
    /// Returns the total number of blobs
    pub const fn total(&self) -> u64 {
        self.remove + self.recover + self.keep
    }
}
#[derive(Debug, Default, Clone, Copy)]
/// Statistics about packs within `prune`
pub struct PackStats {
    /// Number of used packs
    pub used: u64,
    /// Number of partly used packs
    pub partly_used: u64,
    /// Number of unused packs
    pub unused: u64, // this equals to packs-to-remove
    /// Number of packs-to-repack
    pub repack: u64,
    /// Number of packs-to-keep
    pub keep: u64,
}

#[derive(Debug, Default, Clone, Copy, Add)]
/// Statistics about sizes within `prune`
pub struct SizeStats {
    /// Number of used blobs
    pub used: u64,
    /// Number of unused blobs
    pub unused: u64,
    /// Number of blobs to remove
    pub remove: u64,
    /// Number of blobs to repack
    pub repack: u64,
    /// Number of blobs to remove after repacking
    pub repackrm: u64,
}

impl SizeStats {
    /// Returns the total number of blobs
    pub const fn total(&self) -> u64 {
        self.used + self.unused
    }

    /// Returns the total number of blobs after pruning
    pub const fn total_after_prune(&self) -> u64 {
        self.used + self.unused_after_prune()
    }

    /// Returns the total number of unused blobs after pruning
    pub const fn unused_after_prune(&self) -> u64 {
        self.unused - self.remove - self.repackrm
    }
}

/// Statistics about a [`PrunePlan`]
#[derive(Default, Debug)]
pub struct PruneStats {
    /// Statistics about pack count
    pub packs_to_delete: DeleteStats,
    /// Statistics about pack sizes
    pub size_to_delete: DeleteStats,
    /// Statistics about current pack situation
    pub packs: PackStats,
    /// Statistics about blobs in the repository
    pub blobs: BlobTypeMap<SizeStats>,
    /// Statistics about total sizes of blobs in the repository
    pub size: BlobTypeMap<SizeStats>,
    /// Number of unreferenced pack files
    pub packs_unref: u64,
    /// total size of unreferenced pack files
    pub size_unref: u64,
    /// Number of index files
    pub index_files: u64,
    /// Number of index files which will be rebuilt during the prune
    pub index_files_rebuild: u64,
    /// Detailed debug statistics
    pub debug: DebugStats,
}

impl PruneStats {
    /// Compute statistics about blobs of all types
    #[must_use]
    pub fn blobs_sum(&self) -> SizeStats {
        self.blobs
            .values()
            .fold(SizeStats::default(), |acc, x| acc + *x)
    }

    /// Compute total size statistics for blobs of all types
    #[must_use]
    pub fn size_sum(&self) -> SizeStats {
        self.size
            .values()
            .fold(SizeStats::default(), |acc, x| acc + *x)
    }
}

/// prune representation of an index file
#[derive(Debug)]
struct PruneIndex {
    /// The id of the index file
    id: IndexId,
    /// Whether the index file was modified
    modified: bool,
    // The number of packs in the index file
    len: usize,
}

/// Task to be executed by a `PrunePlan` on Packs
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
pub enum PackToDo {
    #[default]
    /// The pack should be kept
    Keep,
    /// The pack should be repacked
    Repack,
    /// The pack should be marked for deletion
    MarkDelete,
    /// The pack should be marked for recovery
    MarkRecover,
    // Keep the deletion Mark
    KeepMarkedDelete,
    // Keep the deletion Mark
    KeepMarkedRecover,
    /// The pack should be recovered
    Recover,
    /// The pack should be deleted
    Delete,
}

impl PackToDo {
    /// ``PackToDo``s which need an index modification
    fn index_modify(self) -> bool {
        !matches!(
            self,
            Self::Keep | Self::KeepMarkedDelete | Self::KeepMarkedRecover
        )
    }
}

/// A pack which is to be pruned
#[derive(Debug, Clone)]
struct PrunePack {
    /// The id of the pack
    id: PackId,
    /// The type of the pack
    blob_type: BlobType,
    /// The size of the pack
    size: u32,
    /// Whether the pack is marked
    mark: IndexMarkType,
    /// The task to be executed on the pack
    to_do: PackToDo,
    /// The time the pack was created
    time: Option<Timestamp>,
    /// The blobs in the pack
    blobs: Vec<IndexBlob>,
    /// the number of the index
    index_num: usize,
}

impl PrunePack {
    /// Create a new `PrunePack` from an `IndexPack`
    ///
    /// # Arguments
    ///
    /// * `p` - The `IndexPack` to create the `PrunePack` from
    /// * `delete_mark` - Whether the pack is marked for deletion
    fn from_index_pack(p: IndexPack, index_num: usize) -> Self {
        Self {
            id: p.id,
            blob_type: p.blob_type(),
            size: p.pack_size(),
            mark: p.mark,
            time: p.time,
            blobs: p.blobs,
            to_do: PackToDo::Keep,
            index_num,
        }
    }

    /// Convert the `PrunePack` into an `IndexPack`.
    fn into_index_pack(self) -> IndexPack {
        IndexPack {
            id: self.id,
            time: self.time,
            size: Some(self.size),
            blobs: self.blobs,
            mark: self.mark,
        }
    }

    /// Set the task to be executed on the pack
    ///
    /// # Arguments
    ///
    /// * `todo` - The task to be executed on the pack
    /// * `pi` - The `PackInfo` of the pack
    /// * `stats` - The `PruneStats` of the `PrunePlan`
    #[allow(clippy::similar_names)]
    fn set_todo(
        &mut self,
        todo: PackToDo,
        pi: &PackInfo,
        status: EnumSet<PackStatus>,
        stats: &mut PruneStats,
    ) {
        let tpe = self.blob_type;
        stats.debug.add(pi, todo, status);
        match todo {
            PackToDo::Keep => {
                stats.packs.keep += 1;
            }
            PackToDo::Repack => {
                stats.packs.repack += 1;
                stats.blobs[tpe].repack += u64::from(pi.unused_blobs + pi.used_blobs);
                stats.blobs[tpe].repackrm += u64::from(pi.unused_blobs);
                stats.size[tpe].repack += u64::from(pi.unused_size + pi.used_size);
                stats.size[tpe].repackrm += u64::from(pi.unused_size);
            }

            PackToDo::MarkDelete => {
                stats.blobs[tpe].remove += u64::from(pi.unused_blobs);
                stats.size[tpe].remove += u64::from(pi.unused_size);
            }
            PackToDo::Recover | PackToDo::MarkRecover => {
                stats.packs_to_delete.recover += 1;
                stats.size_to_delete.recover += u64::from(self.size);
            }
            PackToDo::Delete => {
                stats.packs_to_delete.remove += 1;
                stats.size_to_delete.remove += u64::from(self.size);
            }
            PackToDo::KeepMarkedDelete | PackToDo::KeepMarkedRecover => {
                stats.packs_to_delete.keep += 1;
                stats.size_to_delete.keep += u64::from(self.size);
            }
        }
        self.to_do = todo;
    }

    /// The packs ``PackToDo`` considering the ``instant_delete`` option
    fn todo_with_instant_delete(&self, instant_delete: bool) -> PackToDo {
        match (self.to_do, instant_delete) {
            (PackToDo::MarkDelete | PackToDo::KeepMarkedDelete, true) => PackToDo::Delete,
            (PackToDo::MarkRecover | PackToDo::KeepMarkedRecover, true) => PackToDo::Recover,
            // in all other cases, we keep the original ToDo
            (todo, _) => todo,
        }
    }

    /// Returns whether the pack is compressed
    fn is_compressed(&self) -> bool {
        self.blobs
            .iter()
            .all(|blob| blob.location.uncompressed_length.is_some())
    }
}

/// Reasons why a pack should be repacked
#[derive(PartialEq, Eq, Debug)]
enum RepackReason {
    /// The pack is partly used
    PartlyUsed,
    /// The pack is to be compressed
    ToCompress,
    /// The pack has a size mismatch
    SizeMismatch,
}

/// A plan what should be repacked or removed by a `prune` run
#[derive(Debug)]
pub struct PrunePlan {
    /// The time the plan was created
    time: Zoned,
    /// The ids of the blobs which are used
    used_ids: BTreeMap<BlobId, u8>,
    /// The ids of the existing packs
    existing_packs: BTreeMap<PackId, u32>,
    /// The packs which should be repacked
    repack_candidates: Vec<(PackInfo, EnumSet<PackStatus>, RepackReason, usize)>,
    /// The index files
    index_files: Vec<PruneIndex>,
    /// The packs from the index
    index_packs: Vec<PrunePack>,
    /// `prune` statistics
    pub stats: PruneStats,
    /// some options used to decide about the ``pack_to_do``s for packs
    pack_sizer: BlobTypeMap<PackSizer>,
    opts: PruneOptions,
}

impl PrunePlan {
    /// Create a new `PrunePlan`
    ///
    /// # Arguments
    ///
    /// * `used_ids` - The ids of the blobs which are used
    /// * `existing_packs` - The ids of the existing packs
    /// * `index_files` - The index files
    fn new(
        used_ids: BTreeMap<BlobId, u8>,
        existing_packs: BTreeMap<PackId, u32>,
        index_files: Vec<(IndexId, IndexFile)>,
        opts: PruneOptions,
        pack_sizer: BlobTypeMap<PackSizer>,
    ) -> Self {
        let mut index_packs = Vec::new();
        let mut index_files: Vec<_> = index_files
            .into_iter()
            .enumerate()
            .map(|(index_num, (id, index))| {
                let len = index.len();
                index_packs.extend(
                    index
                        .all_packs()
                        .map(|p| PrunePack::from_index_pack(p, index_num)),
                );

                PruneIndex {
                    id,
                    len,
                    modified: false,
                }
            })
            .collect();

        // sort packs first by id and second by earliest timestamp - so we are cautious not to take a "state" which may not have gotten fully processed.
        index_packs.sort_unstable_by(|a, b| a.id.cmp(&b.id).then(a.time.cmp(&b.time)));

        // Deduplicate by pack id (keeping only the earliest timestamp for duplicates); potentially correcting len and modified flag of the corresponding index_file
        index_packs.dedup_by(|x, p| {
            if x.id == p.id {
                index_files[x.index_num].modified = true;
                index_files[x.index_num].len -= 1;
                true
            } else {
                false
            }
        });

        Self {
            time: Zoned::now(),
            used_ids,
            existing_packs,
            repack_candidates: Vec::new(),
            index_files,
            index_packs,
            stats: PruneStats::default(),
            pack_sizer,
            opts,
        }
    }

    /// Get a `PrunePlan` from the given `PruneOptions`.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The progress bar type
    /// * `S` - The state the repository is in
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to get the `PrunePlan` for
    /// * `opts` - The `PruneOptions` to use
    ///
    /// # Errors
    ///
    /// * If `repack_uncompressed` is set and the repository is a version 1 repository
    /// * If `keep_pack` or `keep_delete` is out of range
    pub fn from_prune_options<S: Open>(
        repo: &Repository<S>,
        opts: &PruneOptions,
    ) -> RusticResult<Self> {
        let be = repo.dbe();

        let version = repo.config().version;

        if version < 2 && opts.repack_uncompressed {
            return Err(RusticError::new(
                ErrorKind::Unsupported,
                "Repacking uncompressed pack is unsupported in Repository version `{config_version}`.",
            )
            .attach_context("config_version", version.to_string()));
        }

        let mut index_files = Vec::new();

        let p = repo.progress_counter("reading index...");
        let mut index_collector = IndexCollector::new(IndexType::OnlyTrees);

        for index in be.stream_all::<IndexFile>(&p)? {
            let (id, index) = index?;
            index_collector.extend(index.clone().all_packs());

            index_files.push((id, index));
        }
        p.finish();

        let (used_ids, total_size) = {
            let index = GlobalIndex::new_from_index(index_collector.into_index());
            let total_size = BlobTypeMap::init(|blob_type| index.total_size(blob_type));
            let used_ids = find_used_blobs(repo, be, &index, &opts.ignore_snaps)?;
            (used_ids, total_size)
        };

        // list existing pack files
        let p = repo.progress_spinner("getting packs from repository...");
        let existing_packs: BTreeMap<_, _> = be
            .list_with_size(FileType::Pack)?
            .into_iter()
            .map(|(id, size)| (PackId::from(id), size))
            .collect();
        p.finish();

        let mut opts = opts.clone();
        if opts.repack_cacheable_only.is_none() {
            // set default value of repack_cacheable_only if it is not set
            opts.repack_cacheable_only = Some(repo.config().is_hot == Some(true));
        }
        let pack_sizer =
            total_size.map(|tpe, size| PackSizer::from_config(repo.config(), tpe, size));

        let mut pruner = Self::new(used_ids, existing_packs, index_files, opts, pack_sizer);
        pruner.count_used_blobs();
        pruner.check()?;
        pruner.decide_packs();
        pruner.decide_repack();
        pruner.check_existing_packs()?;
        pruner.filter_index_files();

        Ok(pruner)
    }

    /// This function counts the number of times a blob is used in the index files.
    fn count_used_blobs(&mut self) {
        for blob in self.index_packs.iter().flat_map(|pack| &pack.blobs) {
            if let Some(count) = self.used_ids.get_mut(&blob.id) {
                // note that duplicates are only counted up to 255. If there are more
                // duplicates, the number is set to 255. This may imply that later on
                // not the "best" pack is chosen to have that blob marked as used.
                *count = count.saturating_add(1);
            }
        }
    }

    /// This function checks whether all used blobs are present in the index files.
    ///
    /// # Errors
    ///
    /// * If a blob is missing
    fn check(&self) -> RusticResult<()> {
        for (id, count) in &self.used_ids {
            if *count == 0 {
                return Err(RusticError::new(
                    ErrorKind::Internal,
                    "Blob ID `{blob_id}` is missing in index files.",
                )
                .attach_context("blob_id", id.to_string())
                .ask_report());
            }
        }

        Ok(())
    }

    /// Decides what to do with the packs
    ///
    /// # Arguments
    ///
    /// * `keep_pack` - The minimum duration to keep packs before repacking or removing
    /// * `keep_delete` - The minimum duration to keep packs marked for deletion
    /// * `repack_cacheable_only` - Whether to only repack cacheable packs
    /// * `repack_uncompressed` - Whether to repack packs containing uncompressed blobs
    /// * `repack_all` - Whether to repack all packs
    /// * `pack_sizer` - The `PackSizer` for the packs
    fn decide_packs(&mut self) {
        // take out index files as we will process it - will be re-inserted at the end
        let mut index_packs = std::mem::take(&mut self.index_packs);

        // sort packs such that marked packs come first
        index_packs.sort_unstable_by_key(|pack| Reverse(pack.mark));

        // We first process all marked packs then the unmarked ones:
        // - first processed packs are more likely to have all blobs seen as unused
        // - if marked packs have used blob but these blobs are all present in
        //   unmarked packs, we want to perform the deletion!
        for (pack_num, pack) in index_packs.iter_mut().enumerate() {
            let pi = PackInfo::from_pack(pack, &mut self.used_ids);
            //update used/unused stats
            self.stats.blobs[pi.blob_type].used += u64::from(pi.used_blobs);
            self.stats.blobs[pi.blob_type].unused += u64::from(pi.unused_blobs);
            self.stats.size[pi.blob_type].used += u64::from(pi.used_size);
            self.stats.size[pi.blob_type].unused += u64::from(pi.unused_size);
            let mut status = EnumSet::empty();
            if pi.used_blobs > 0 {
                _ = status.insert(PackStatus::HasUsedBlobs);
            }
            if pi.unused_blobs > 0 {
                _ = status.insert(PackStatus::HasUnusedBlobs);
            }

            if pack.mark.has_mark() {
                self.decide_marked_pack(pack, pi, status);
            } else {
                self.decide_unmarked_pack(pack_num, pack, pi, status);
            }
        }
        // re-insert index_files
        self.index_packs = index_packs;
    }

    fn decide_unmarked_pack(
        &mut self,
        pack_num: usize,
        pack: &mut PrunePack,
        pi: PackInfo,
        mut status: EnumSet<PackStatus>,
    ) {
        // Various checks to determine if packs need to be kept

        // too_young is set if the mark is not old enough. If the pack time is unset, it is set to false
        let too_young = pack.time > Some(self.time.saturating_sub(self.opts.keep_pack).into());
        if too_young {
            _ = status.insert(PackStatus::TooYoung);
        }
        let keep_uncacheable =
            self.opts.repack_cacheable_only.unwrap_or_default() && !pack.blob_type.is_cacheable();

        let to_compress = self.opts.repack_uncompressed && !pack.is_compressed();
        if to_compress {
            _ = status.insert(PackStatus::NotCompressed);
        }
        let size_mismatch = !self.pack_sizer[pack.blob_type].size_ok(pack.size);
        if self.pack_sizer[pack.blob_type].is_too_small(pack.size) {
            _ = status.insert(PackStatus::TooSmall);
        }
        if self.pack_sizer[pack.blob_type].is_too_large(pack.size) {
            _ = status.insert(PackStatus::TooLarge);
        }
        match (pi.used_blobs, pi.unused_blobs) {
            (0, _) => {
                // unused pack
                self.stats.packs.unused += 1;
                _ = status.insert(PackStatus::HasUnusedBlobs);
                if too_young {
                    // keep packs which are too young
                    pack.set_todo(PackToDo::Keep, &pi, status, &mut self.stats);
                } else {
                    pack.set_todo(PackToDo::MarkDelete, &pi, status, &mut self.stats);
                }
            }
            (1.., 0) => {
                // used pack
                self.stats.packs.used += 1;
                _ = status.insert(PackStatus::HasUsedBlobs);
                if too_young || keep_uncacheable {
                    pack.set_todo(PackToDo::Keep, &pi, status, &mut self.stats);
                } else if to_compress || self.opts.repack_all {
                    self.repack_candidates
                        .push((pi, status, RepackReason::ToCompress, pack_num));
                } else if size_mismatch {
                    self.repack_candidates
                        .push((pi, status, RepackReason::SizeMismatch, pack_num));
                } else {
                    pack.set_todo(PackToDo::Keep, &pi, status, &mut self.stats);
                }
            }

            (1.., 1..) => {
                // partly used pack
                self.stats.packs.partly_used += 1;
                status.insert_all(PackStatus::HasUsedBlobs | PackStatus::HasUnusedBlobs);

                if too_young || keep_uncacheable {
                    // keep packs which are too young and non-cacheable packs if requested
                    pack.set_todo(PackToDo::Keep, &pi, status, &mut self.stats);
                } else {
                    // other partly used pack => candidate for repacking
                    self.repack_candidates
                        .push((pi, status, RepackReason::PartlyUsed, pack_num));
                }
            }
        }
    }

    fn decide_marked_pack(
        &mut self,
        pack: &mut PrunePack,
        pi: PackInfo,
        mut status: EnumSet<PackStatus>,
    ) {
        _ = status.insert(PackStatus::Marked);
        // too_young is set if the mark is not old enough. If the pack time is unset, it is set to true
        let too_young = pack.time.is_none_or(|pack_time| {
            pack_time > self.time.saturating_sub(self.opts.keep_marked).into()
        });
        if too_young {
            _ = status.insert(PackStatus::TooYoung);
        }
        let pack_to_do = match (pack.mark, pi.used_blobs) {
            (IndexMarkType::None, _) => panic!("not possible"),
            (IndexMarkType::Delete, 1..) => PackToDo::MarkRecover, // was wrong marked; correct
            (IndexMarkType::Recover, 0) => PackToDo::MarkDelete,   // was wrong marked; correct
            (IndexMarkType::Delete, 0) if too_young => PackToDo::KeepMarkedDelete,
            (IndexMarkType::Recover, 1..) if too_young => PackToDo::KeepMarkedRecover,
            (IndexMarkType::Delete, 0) => PackToDo::Delete,
            (IndexMarkType::Recover, 1..) => PackToDo::Recover,
        };
        pack.set_todo(pack_to_do, &pi, status, &mut self.stats);
    }

    /// Decides if packs should be repacked
    ///
    /// # Arguments
    ///
    /// * `max_repack` - The maximum size of packs to repack
    /// * `max_unused` - The maximum size of unused blobs
    /// * `repack_uncompressed` - Whether to repack packs containing uncompressed blobs
    /// * `no_resize` - Whether to resize packs
    /// * `pack_sizer` - The `PackSizer` for the packs
    fn decide_repack(&mut self) {
        let max_unused = match (
            self.opts.repack_uncompressed || self.opts.repack_all,
            self.opts.max_unused,
        ) {
            (true, _) => 0,
            (false, LimitOption::Unlimited) => u64::MAX,
            (false, LimitOption::Size(size)) => size.as_u64(),
            // if percentag is given, we want to have
            // unused <= p/100 * size_after = p/100 * (size_used + unused)
            // which equals (1 - p/100) * unused <= p/100 * size_used
            (false, LimitOption::Percentage(p)) => (p * self.stats.size_sum().used) / (100 - p),
        };

        let max_repack = match self.opts.max_repack {
            LimitOption::Unlimited => u64::MAX,
            LimitOption::Size(size) => size.as_u64(),
            LimitOption::Percentage(p) => (p * self.stats.size_sum().total()) / 100,
        };

        self.repack_candidates.sort_unstable_by_key(|rc| rc.0);
        let mut resize_packs = BlobTypeMap::<Vec<_>>::default();
        let mut do_repack = BlobTypeMap::default();
        let mut repack_size = BlobTypeMap::<u64>::default();

        for (pi, status, repack_reason, pack_num) in std::mem::take(&mut self.repack_candidates) {
            let pack = &mut self.index_packs[pack_num];
            let blob_type = pi.blob_type;

            let total_repack_size: u64 = repack_size.into_values().sum();
            if total_repack_size + u64::from(pi.used_size) >= max_repack
                || (self.stats.size_sum().unused_after_prune() < max_unused
                    && repack_reason == RepackReason::PartlyUsed
                    && blob_type == BlobType::Data)
                || (repack_reason == RepackReason::SizeMismatch && self.opts.no_resize)
            {
                pack.set_todo(PackToDo::Keep, &pi, status, &mut self.stats);
            } else if repack_reason == RepackReason::SizeMismatch {
                resize_packs[blob_type].push((pi, status, pack_num));
                repack_size[blob_type] += u64::from(pi.used_size);
            } else {
                pack.set_todo(PackToDo::Repack, &pi, status, &mut self.stats);
                repack_size[blob_type] += u64::from(pi.used_size);
                do_repack[blob_type] = true;
            }
        }
        for (blob_type, resize_packs) in resize_packs {
            // packs in resize_packs are only repacked if we anyway repack this blob type or
            // if the target pack size is reached for the blob type.
            let todo = if do_repack[blob_type]
                || repack_size[blob_type] > u64::from(self.pack_sizer[blob_type].pack_size())
            {
                PackToDo::Repack
            } else {
                PackToDo::Keep
            };
            for (pi, status, pack_num) in resize_packs {
                let pack = &mut self.index_packs[pack_num];
                pack.set_todo(todo, &pi, status, &mut self.stats);
            }
        }
    }

    /// Checks if the existing packs are ok
    ///
    /// # Errors
    ///
    /// * If a pack is undecided
    /// * If the size of a pack does not match
    /// * If a pack does not exist
    fn check_existing_packs(&mut self) -> RusticResult<()> {
        for pack in &self.index_packs {
            let existing_size = self.existing_packs.remove(&pack.id);

            // TODO: Unused Packs which don't exist (i.e. only existing in index)
            let check_size = || {
                match existing_size {
                    Some(size) if size == pack.size => Ok(()), // size is ok => continue
                    Some(size) => Err(RusticError::new(
                        ErrorKind::Internal,
                        "Pack size `{size_in_pack_real}` of id `{pack_id}` does not match the expected size `{size_in_index_expected}` in the index file. ",
                    )
                    .attach_context("pack_id", pack.id.to_string())
                    .attach_context("size_in_index_expected", pack.size.to_string())
                    .attach_context("size_in_pack_real", size.to_string())
                    .ask_report()),
                    None => Err(RusticError::new(ErrorKind::Internal, "Pack `{pack_id}` does not exist.")
                        .attach_context("pack_id", pack.id.to_string())
                        .ask_report()),
                }
            };

            match pack.to_do {
                PackToDo::Keep | PackToDo::Recover => {
                    for blob in &pack.blobs {
                        _ = self.used_ids.remove(&blob.id);
                    }
                    check_size()?;
                }
                PackToDo::Repack => {
                    check_size()?;
                }
                PackToDo::MarkDelete
                | PackToDo::Delete
                | PackToDo::KeepMarkedDelete
                | PackToDo::KeepMarkedRecover
                | PackToDo::MarkRecover => {}
            }
        }

        // all remaining packs in existing_packs are unreferenced packs
        for size in self.existing_packs.values() {
            self.stats.size_unref += u64::from(*size);
        }
        self.stats.packs_unref = self.existing_packs.len() as u64;

        Ok(())
    }

    /// Filter out index files which do not need processing
    ///
    /// # Arguments
    ///
    /// * `instant_delete` - Whether to instantly delete unreferenced packs
    fn filter_index_files(&mut self) {
        self.stats.index_files = self.index_files.len() as u64;

        // set modify flag for index_files which need modification
        for pack in &self.index_packs {
            if pack
                .todo_with_instant_delete(self.opts.instant_delete)
                .index_modify()
            {
                self.index_files[pack.index_num].modified = true;
            }
        }

        // also process index files which are too small (i.e. rebuild them) - but don't do this if we have only a single too-small index file
        let mut any_must_modify = false;
        for index in &mut self.index_files {
            any_must_modify |= index.modified;
            index.modified |= index.len < constants::MIN_INDEX_LEN;
        }
        if !any_must_modify && self.index_files.len() == 1 {
            // only one index file to process but only because it is too small => we do nothing here
            self.index_files.clear();
            self.index_packs.clear();
        }

        // only keep packs for index_files which will be modified
        self.index_packs
            .retain(|p| self.index_files[p.index_num].modified);

        // only keep index files which will be modified
        self.index_files.retain(|index| index.modified);
        self.stats.index_files_rebuild = self.index_files.len() as u64;

        // TODO: Sort packs such that files with deletes come first and files with
        // repacks come at end
    }

    /// Get the list of packs-to-repack from the [`PrunePlan`].
    #[must_use]
    pub fn repack_packs(&self) -> Vec<PackId> {
        self.index_packs
            .iter()
            .filter(|pack| pack.to_do == PackToDo::Repack)
            .map(|pack| pack.id)
            .collect()
    }
}

/// Perform the pruning on the given repository.
///
/// # Arguments
///
/// * `repo` - The repository to prune
/// * `opts` - The options for the pruning
/// * `prune_plan` - The plan for the pruning
///
/// # Errors
///
/// * If the repository is in append-only mode
/// * If a pack has no decision
///
/// # Returns
///
/// * `Ok(())` - If the pruning was successful
///
/// # Panics
///
/// TODO! In weird circumstances, should be fixed.
#[allow(clippy::significant_drop_tightening)]
#[allow(clippy::too_many_lines)]
pub(crate) fn prune_repository<S: Open>(
    repo: &Repository<S>,
    opts: &PruneOptions,
    prune_plan: PrunePlan,
) -> RusticResult<()> {
    if repo.config().append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Pruning is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }
    repo.warm_up_wait(prune_plan.repack_packs().into_iter())?;
    let be = repo.dbe();
    let prune_time = prune_plan.time.timestamp();

    let mut indexer = Indexer::new_unindexed(be.clone());
    // mark unreferenced packs for deletion
    if !prune_plan.existing_packs.is_empty() {
        if opts.instant_delete {
            let p = repo.progress_counter("removing unindexed packs...");
            let existing_packs: Vec<_> = prune_plan.existing_packs.into_keys().collect();
            be.delete_list(true, existing_packs.iter(), p)?;
        } else {
            let p = repo.progress_counter("marking unneeded unindexed pack files for deletion...");
            p.set_length(
                prune_plan
                    .existing_packs
                    .len()
                    .try_into()
                    .unwrap_or_default(),
            );
            for (id, size) in prune_plan.existing_packs {
                let pack = IndexPack {
                    id,
                    size: Some(size),
                    time: Some(prune_time),
                    blobs: Vec::new(),
                    mark: IndexMarkType::Delete,
                };
                indexer.add(pack)?;
                p.inc(1);
            }
            p.finish();
        }
    }

    if prune_plan.index_files.is_empty() {
        info!("nothing to do!");
        return Ok(());
    }

    let indexes_remove: Vec<_> = prune_plan
        .index_files
        .iter()
        .map(|index| index.id)
        .collect();

    let early_delete_index = opts.early_delete_index && opts.instant_delete;

    // remove old index files early if requested
    if !indexes_remove.is_empty() && early_delete_index {
        let p = repo.progress_counter("removing old index files...");
        be.delete_list(true, indexes_remove.iter(), p)?;
    }

    let mut tree_packs_remove = Vec::new();
    let mut data_packs_remove = Vec::new();

    let mut delete_pack = |pack: &PrunePack| {
        // delete pack
        match pack.blob_type {
            BlobType::Data => data_packs_remove.push(pack.id),
            BlobType::Tree => tree_packs_remove.push(pack.id),
        }
    };

    let mut used_ids = prune_plan.used_ids;
    let mut repack_packs = Vec::new();

    // process packs and write new index files
    let p = repo.progress_counter("rebuilding index...");
    p.set_length(u64::try_from(prune_plan.index_files.len()).unwrap_or_default());
    for mut pack in prune_plan.index_packs {
        // for instant_delete we choose according ToDos
        let pack_to_do = pack.todo_with_instant_delete(opts.instant_delete);
        match pack_to_do {
            PackToDo::Keep => {
                // keep pack: add to new index; correct time if not set
                let mut pack = pack.into_index_pack();
                pack.set_time_if_unset(prune_time);
                pack.mark = IndexMarkType::None;
                indexer.add(pack)?;
            }
            PackToDo::Repack => {
                if opts.instant_delete {
                    delete_pack(&pack);
                } else {
                    // mark pack for removal
                    let mut pack = pack.clone().into_index_pack();
                    pack.time = Some(prune_time);
                    pack.mark = IndexMarkType::Delete;
                    indexer.add(pack)?;
                }
                pack.blobs
                    .retain(|blob| used_ids.remove(&blob.id).is_some()); // don't save duplicate blobs
                // sort blobs to later allow coalescing
                pack.blobs.sort_unstable();
                repack_packs.push(pack);
            }
            PackToDo::MarkDelete => {
                // mark pack
                let mut pack = pack.into_index_pack();
                pack.time = Some(prune_time);
                pack.mark = IndexMarkType::Delete;
                indexer.add(pack)?;
            }
            PackToDo::MarkRecover => {
                // mark pack
                let mut pack = pack.into_index_pack();
                pack.time = Some(prune_time);
                pack.mark = IndexMarkType::Recover;
                indexer.add(pack)?;
            }
            PackToDo::KeepMarkedDelete => {
                // keep pack: add to new index; keep the timestamp.
                // Note the timestamp shouldn't be None here, however if it is not not set, use the current time to heal the entry!
                let mut pack = pack.into_index_pack();
                pack.set_time_if_unset(prune_time);
                pack.mark = IndexMarkType::Delete;
                indexer.add(pack)?;
            }
            PackToDo::KeepMarkedRecover => {
                // keep pack: add to new index; keep the timestamp.
                // Note the timestamp shouldn't be None here, however if it is not not set, use the current time to heal the entry!
                let mut pack = pack.into_index_pack();
                pack.set_time_if_unset(prune_time);
                pack.mark = IndexMarkType::Recover;
                indexer.add(pack)?;
            }
            PackToDo::Recover => {
                // recover pack: add to new index in section packs
                let mut pack = pack.into_index_pack();
                pack.time = Some(prune_time);
                pack.mark = IndexMarkType::None;
                indexer.add(pack)?;
            }
            PackToDo::Delete => delete_pack(&pack),
        }
    }
    p.finish();

    if repack_packs.is_empty() {
        indexer.finalize()?;
    } else {
        let p = repo.progress_bytes("repacking...");
        p.set_length(prune_plan.stats.size_sum().repack - prune_plan.stats.size_sum().repackrm);
        let indexer = indexer.into_shared();

        // Calculate an approximation of sizes after pruning.
        // The size actually is:
        // total_size_of_all_blobs + total_size_of_pack_headers + #packs * pack_overhead
        // This is hard/impossible to compute because:
        // - the size of blobs can change during repacking if compression is changed
        // - the size of pack headers depends on whether blobs are compressed or not
        // - we don't know the number of packs generated by repacking
        // So, we simply use the current size of the blobs and an estimation of the pack
        // header size.

        let size_after_prune = BlobTypeMap::init(|blob_type| {
            prune_plan.stats.size[blob_type].total_after_prune()
                + prune_plan.stats.blobs[blob_type].total_after_prune()
                    * u64::from(HeaderEntry::ENTRY_LEN_COMPRESSED)
        });

        // use a fixed pack size corresponding to the estimated size after pruning.
        let pack_sizer = size_after_prune.map(|blob_type, size| {
            PackSizer::fixed(PackSizer::from_config(repo.config(), blob_type, size).pack_size())
        });

        let tree_repacker = BlobCopier::new(
            be.clone(),
            be.clone(),
            BlobType::Tree,
            indexer.clone(),
            pack_sizer[BlobType::Tree],
        )?;

        let data_repacker = BlobCopier::new(
            be.clone(),
            be.clone(),
            BlobType::Data,
            indexer.clone(),
            pack_sizer[BlobType::Data],
        )?;

        // write new pack files and index files
        repack_packs
            .into_par_iter()
            .try_for_each(|pack| -> RusticResult<_> {
                let repacker = match pack.blob_type {
                    BlobType::Data => &data_repacker,
                    BlobType::Tree => &tree_repacker,
                };
                let blob_chunks: Vec<_> = pack
                    .blobs
                    .into_iter()
                    .map(|blob| BlobLocations::from_blob_location(blob.location, blob.id))
                    .coalesce(BlobLocations::coalesce)
                    .map(|locations| CopyPackBlobs {
                        pack_id: pack.id,
                        locations,
                    })
                    .collect();

                // TODO: repack in parallel
                for blobs in blob_chunks {
                    if opts.fast_repack {
                        repacker.copy_fast(blobs, &p)?;
                    } else {
                        repacker.copy(blobs, &p)?;
                    }
                }
                Ok(())
            })?;
        _ = tree_repacker.finalize()?;
        _ = data_repacker.finalize()?;
        indexer.write().unwrap().finalize()?;
        p.finish();
    }

    // remove old index files first as they may reference pack files which are removed soon.
    if !indexes_remove.is_empty() && !early_delete_index {
        let p = repo.progress_counter("removing old index files...");
        be.delete_list(true, indexes_remove.iter(), p)?;
    }

    if !data_packs_remove.is_empty() {
        let p = repo.progress_counter("removing old data packs...");
        be.delete_list(false, data_packs_remove.iter(), p)?;
    }

    if !tree_packs_remove.is_empty() {
        let p = repo.progress_counter("removing old tree packs...");
        be.delete_list(true, tree_packs_remove.iter(), p)?;
    }

    Ok(())
}

/// `PackInfo` contains information about a pack which is needed to decide what to do with the pack.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
struct PackInfo {
    /// What type of blobs are in the pack
    blob_type: BlobType,
    /// The number of used blobs in the pack
    used_blobs: u16,
    /// The number of unused blobs in the pack
    unused_blobs: u16,
    /// The size of the used blobs in the pack
    used_size: u32,
    /// The size of the unused blobs in the pack
    unused_size: u32,
}

impl PartialOrd<Self> for PackInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        // first order by blob type such that tree packs are picked first
        self.blob_type.cmp(&other.blob_type).then(
            // then order such that packs with highest
            // ratio unused/used space are picked first.
            // This is equivalent to ordering by unused / total space.
            (u64::from(other.unused_size) * u64::from(self.used_size))
                .cmp(&(u64::from(self.unused_size) * u64::from(other.used_size))),
        )
    }
}

impl PackInfo {
    /// Create a `PackInfo` from a `PrunePack`.
    ///
    /// # Arguments
    ///
    /// * `pack` - The `PrunePack` to create the `PackInfo` from
    /// * `used_ids` - The `BTreeMap` of used ids
    fn from_pack(pack: &PrunePack, used_ids: &mut BTreeMap<BlobId, u8>) -> Self {
        let mut pi = Self {
            blob_type: pack.blob_type,
            used_blobs: 0,
            unused_blobs: 0,
            used_size: 0,
            unused_size: 0,
        };

        // We search all blobs in the pack for needed ones. We do this by already marking
        // and decreasing the used blob counter for the processed blobs. If the counter
        // was decreased to 0, the blob and therefore the pack is actually used.
        // Note that by this processing, we are also able to handle duplicate blobs within a pack
        // correctly.
        // If we found a needed blob, we stop and process the information that the pack is actually needed.
        let first_needed = pack.blobs.iter().position(|blob| {
            let length = blob.location.length;
            match used_ids.get_mut(&blob.id) {
                None | Some(0) => {
                    pi.unused_size += length;
                    pi.unused_blobs += 1;
                }
                Some(count) => {
                    // decrease counter
                    *count -= 1;
                    if *count == 0 {
                        // blob is actually needed
                        pi.used_size += length;
                        pi.used_blobs += 1;
                        return true; // break the search
                    }
                    // blob is not needed
                    pi.unused_size += length;
                    pi.unused_blobs += 1;
                }
            }
            false // continue with next blob
        });

        if let Some(first_needed) = first_needed {
            // The pack is actually needed.
            // We reprocess the blobs up to the first needed one and mark all blobs which are generally needed as used.
            for blob in &pack.blobs[..first_needed] {
                match used_ids.get_mut(&blob.id) {
                    None | Some(0) => {} // already correctly marked
                    Some(count) => {
                        // remark blob as used
                        pi.unused_size -= blob.location.length;
                        pi.unused_blobs -= 1;
                        pi.used_size += blob.location.length;
                        pi.used_blobs += 1;
                        *count = 0; // count = 0 indicates to other packs that the blob is not needed anymore.
                    }
                }
            }
            // Then we process the remaining blobs and mark all blobs which are generally needed as used in this blob
            for blob in &pack.blobs[first_needed + 1..] {
                match used_ids.get_mut(&blob.id) {
                    None | Some(0) => {
                        pi.unused_size += blob.location.length;
                        pi.unused_blobs += 1;
                    }
                    Some(count) => {
                        // blob is used in this pack
                        pi.used_size += blob.location.length;
                        pi.used_blobs += 1;
                        *count = 0; // count = 0 indicates to other packs that the blob is not needed anymore.
                    }
                }
            }
        }

        pi
    }
}

/// Find used blobs in repo and return a map of used ids.
///
/// # Arguments
///
/// * `index` - The index to use
/// * `ignore_snaps` - The snapshots to ignore
/// * `pb` - The progress bars
///
/// # Errors
///
// TODO!: add errors!
fn find_used_blobs<S>(
    repo: &Repository<S>,
    be: &impl DecryptReadBackend,
    index: &impl ReadGlobalIndex,
    ignore_snaps: &[SnapshotId],
) -> RusticResult<BTreeMap<BlobId, u8>> {
    let ignore_snaps: BTreeSet<_> = ignore_snaps.iter().collect();

    let p = repo.progress_counter("reading snapshots...");
    let list: Vec<_> = be
        .list(FileType::Snapshot)?
        .into_iter()
        .map(SnapshotId::from)
        .filter(|id| !ignore_snaps.contains(&id))
        .collect();
    let snap_trees: Vec<_> = be
        .stream_list::<SnapshotFile>(list, &p)?
        .into_iter()
        .map_ok(|(_, snap)| snap.tree)
        .try_collect()?;
    p.finish();

    let mut ids: BTreeMap<_, _> = snap_trees
        .iter()
        .map(|id| (BlobId::from(**id), 0))
        .collect();
    let p = repo.progress_counter("finding used blobs...");

    let mut tree_streamer = TreeStreamerOnce::new(be, index, snap_trees, p)?;
    while let Some(item) = tree_streamer.next().transpose()? {
        let (_, tree) = item;
        collect_used_node_blobs(tree.nodes, &mut ids);
    }

    Ok(ids)
}

fn collect_used_node_blobs(nodes: impl IntoIterator<Item = Node>, ids: &mut BTreeMap<BlobId, u8>) {
    for node in nodes {
        match node.node_type {
            NodeType::File => {
                ids.extend(
                    node.content
                        .iter()
                        .flatten()
                        .map(|id| (BlobId::from(**id), 0)),
                );
            }
            NodeType::Dir => {
                if let Some(subtree) = node.subtree {
                    _ = ids.insert(BlobId::from(*subtree), 0);
                }
            }
            _ => {} // nothing to do
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::node::Node;
    use crate::blob::tree::TreeId;

    #[test]
    fn test_collect_used_node_blobs_dir_without_subtree_does_not_panic() {
        let mut ids = BTreeMap::new();
        let node = Node {
            name: "corrupted_dir".to_string(),
            node_type: NodeType::Dir,
            subtree: None,
            ..Default::default()
        };

        collect_used_node_blobs([node], &mut ids);
        assert!(ids.is_empty());
    }

    #[test]
    fn test_collect_used_node_blobs_dir_with_subtree() {
        let mut ids = BTreeMap::new();
        let tree_id = TreeId::default();
        let node = Node {
            name: "valid_dir".to_string(),
            node_type: NodeType::Dir,
            subtree: Some(tree_id),
            ..Default::default()
        };

        collect_used_node_blobs([node], &mut ids);
        assert!(ids.contains_key(&BlobId::from(*tree_id)));
    }
}
