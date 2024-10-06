use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Display},
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::{DateTime, Duration, Local};
use derivative::Derivative;
use derive_setters::Setters;
use dunce::canonicalize;
use gethostname::gethostname;
use itertools::Itertools;
use log::info;
use path_dedot::ParseDot;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};

use crate::{
    backend::{decrypt::DecryptReadBackend, FileType, FindInBackend},
    blob::tree::TreeId,
    error::{RusticError, RusticErrorKind, RusticResult, SnapshotFileErrorKind},
    impl_repofile,
    progress::Progress,
    repofile::RepoFile,
    Id,
};

#[cfg(feature = "clap")]
use clap::ValueHint;

/// Options for creating a new [`SnapshotFile`] structure for a new backup snapshot.
///
/// This struct derives [`serde::Deserialize`] allowing to use it in config files.
///
/// # Features
///
/// * With the feature `merge` enabled, this also derives [`conflate::Merge`] to allow merging [`SnapshotOptions`] from multiple sources.
/// * With the feature `clap` enabled, this also derives [`clap::Parser`] allowing it to be used as CLI options.
///
/// # Note
///
/// The preferred way is to use [`SnapshotFile::from_options`] to create a SnapshotFile for a new backup.
#[serde_as]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Deserialize, Serialize, Clone, Default, Debug, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
pub struct SnapshotOptions {
    /// Label snapshot with given label
    #[cfg_attr(feature = "clap", clap(long, value_name = "LABEL"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub label: Option<String>,

    /// Tags to add to snapshot (can be specified multiple times)
    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[cfg_attr(feature = "clap", clap(long = "tag", value_name = "TAG[,TAG,..]"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub tags: Vec<StringList>,

    /// Add description to snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DESCRIPTION"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub description: Option<String>,

    /// Add description to snapshot from file
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "FILE", conflicts_with = "description", value_hint = ValueHint::FilePath)
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub description_from: Option<PathBuf>,

    /// Set the backup time manually
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub time: Option<DateTime<Local>>,

    /// Mark snapshot as uneraseable
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "delete_after"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub delete_never: bool,

    /// Mark snapshot to be deleted after given duration (e.g. 10d)
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub delete_after: Option<humantime::Duration>,

    /// Set the host name manually
    #[cfg_attr(feature = "clap", clap(long, value_name = "NAME"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub host: Option<String>,

    /// Set the backup command manually
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub command: Option<String>,
}

impl SnapshotOptions {
    /// Add tags to this [`SnapshotOptions`]
    ///
    /// # Arguments
    ///
    /// * `tag` - The tag to add
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::NonUnicodeTag`] - If the tag is not valid unicode
    ///
    /// # Returns
    ///
    /// The modified [`SnapshotOptions`]
    ///
    /// [`SnapshotFileErrorKind::NonUnicodeTag`]: crate::error::SnapshotFileErrorKind::NonUnicodeTag
    pub fn add_tags(mut self, tag: &str) -> RusticResult<Self> {
        self.tags.push(StringList::from_str(tag)?);
        Ok(self)
    }

    /// Create a new [`SnapshotFile`] using this `SnapshotOption`s
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::NonUnicodeHostname`] - If the hostname is not valid unicode
    ///
    /// # Returns
    ///
    /// The new [`SnapshotFile`]
    ///
    /// [`SnapshotFileErrorKind::NonUnicodeHostname`]: crate::error::SnapshotFileErrorKind::NonUnicodeHostname
    pub fn to_snapshot(&self) -> RusticResult<SnapshotFile> {
        SnapshotFile::from_options(self)
    }
}

/// Summary information about a snapshot.
///
/// This is an extended version of the summaryOutput structure of restic in
/// restic/internal/ui/backup$/json.go
#[derive(Serialize, Deserialize, Debug, Clone, Derivative)]
#[serde(default)]
#[derivative(Default)]
#[non_exhaustive]
pub struct SnapshotSummary {
    /// New files compared to the last (i.e. parent) snapshot
    pub files_new: u64,

    /// Changed files compared to the last (i.e. parent) snapshot
    pub files_changed: u64,

    /// Unchanged files compared to the last (i.e. parent) snapshot
    pub files_unmodified: u64,

    /// Total processed files
    pub total_files_processed: u64,

    /// Total size of all processed files
    pub total_bytes_processed: u64,

    /// New directories compared to the last (i.e. parent) snapshot
    pub dirs_new: u64,

    /// Changed directories compared to the last (i.e. parent) snapshot
    pub dirs_changed: u64,

    /// Unchanged directories compared to the last (i.e. parent) snapshot
    pub dirs_unmodified: u64,

    /// Total processed directories
    pub total_dirs_processed: u64,

    /// Total number of data blobs added by this snapshot
    pub total_dirsize_processed: u64,

    /// Total size of all processed dirs
    pub data_blobs: u64,

    /// Total number of tree blobs added by this snapshot
    pub tree_blobs: u64,

    /// Total uncompressed bytes added by this snapshot
    pub data_added: u64,

    /// Total bytes added to the repository by this snapshot
    pub data_added_packed: u64,

    /// Total uncompressed bytes (new/changed files) added by this snapshot
    pub data_added_files: u64,

    /// Total bytes for new/changed files added to the repository by this snapshot
    pub data_added_files_packed: u64,

    /// Total uncompressed bytes (new/changed directories) added by this snapshot
    pub data_added_trees: u64,

    /// Total bytes (new/changed directories) added to the repository by this snapshot
    pub data_added_trees_packed: u64,

    /// The command used to make this backup
    pub command: String,

    /// Start time of the backup.
    ///
    /// # Note
    ///
    /// This may differ from the snapshot `time`.
    #[derivative(Default(value = "Local::now()"))]
    pub backup_start: DateTime<Local>,

    /// The time that the backup has been finished.
    #[derivative(Default(value = "Local::now()"))]
    pub backup_end: DateTime<Local>,

    /// Total duration of the backup in seconds, i.e. the time between `backup_start` and `backup_end`
    pub backup_duration: f64,

    /// Total duration that the rustic command ran in seconds
    pub total_duration: f64,
}

impl SnapshotSummary {
    /// Create a new [`SnapshotSummary`].
    ///
    /// # Arguments
    ///
    /// * `snap_time` - The time of the snapshot
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::OutOfRange`] - If the time is not in the range of `Local::now()`
    ///
    /// [`SnapshotFileErrorKind::OutOfRange`]: crate::error::SnapshotFileErrorKind::OutOfRange
    pub(crate) fn finalize(&mut self, snap_time: DateTime<Local>) -> RusticResult<()> {
        let end_time = Local::now();
        self.backup_duration = (end_time - self.backup_start)
            .to_std()
            .map_err(SnapshotFileErrorKind::OutOfRange)?
            .as_secs_f64();
        self.total_duration = (end_time - snap_time)
            .to_std()
            .map_err(SnapshotFileErrorKind::OutOfRange)?
            .as_secs_f64();
        self.backup_end = end_time;
        Ok(())
    }
}

/// Options for deleting snapshots.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Derivative, Copy)]
#[derivative(Default)]
pub enum DeleteOption {
    /// No delete option set.
    #[derivative(Default)]
    NotSet,
    /// This snapshot should be never deleted (remove-protection).
    Never,
    /// Remove this snapshot after the given timestamp, but prevent removing it before.
    After(DateTime<Local>),
}

impl DeleteOption {
    /// Returns whether the delete option is set to `NotSet`.
    const fn is_not_set(&self) -> bool {
        matches!(self, Self::NotSet)
    }
}

impl_repofile!(SnapshotId, FileType::Snapshot, SnapshotFile);

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Derivative)]
#[derivative(Default)]
/// A [`SnapshotFile`] is the repository representation of the snapshot metadata saved in a repository.
///
/// It is usually saved in the repository under `snapshot/<ID>`
///
/// # Note
///
/// [`SnapshotFile`] implements [`Eq`], [`PartialEq`], [`Ord`], [`PartialOrd`] by comparing only the `time` field.
/// If you need another ordering, you have to implement that yourself.
pub struct SnapshotFile {
    #[derivative(Default(value = "Local::now()"))]
    /// Timestamp of this snapshot
    pub time: DateTime<Local>,

    /// Program identifier and its version that have been used to create this snapshot.
    #[derivative(Default(
        value = "\"rustic \".to_string() + option_env!(\"PROJECT_VERSION\").unwrap_or(env!(\"CARGO_PKG_VERSION\"))"
    ))]
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub program_version: String,

    /// The Id of the parent snapshot that this snapshot has been based on
    pub parent: Option<SnapshotId>,

    /// The tree blob id where the contents of this snapshot are stored
    pub tree: TreeId,

    /// Label for the snapshot
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,

    /// The list of paths contained in this snapshot
    pub paths: StringList,

    /// The hostname of the device on which the snapshot has been created
    #[serde(default)]
    pub hostname: String,

    /// The username that started the backup run
    #[serde(default)]
    pub username: String,

    /// The uid of the username that started the backup run
    #[serde(default)]
    pub uid: u32,

    /// The gid of the username that started the backup run
    #[serde(default)]
    pub gid: u32,

    /// A list of tags for this snapshot
    #[serde(default)]
    pub tags: StringList,

    /// The original Id of this snapshot. This is stored when the snapshot is modified.
    pub original: Option<SnapshotId>,

    /// Options for deletion of the snapshot
    #[serde(default, skip_serializing_if = "DeleteOption::is_not_set")]
    pub delete: DeleteOption,

    /// Summary information about the backup run
    pub summary: Option<SnapshotSummary>,

    /// A description of what is contained in this snapshot
    pub description: Option<String>,

    /// The snapshot Id (not stored within the JSON)
    #[serde(default, skip_serializing_if = "Id::is_null")]
    pub id: SnapshotId,
}

impl SnapshotFile {
    /// Create a [`SnapshotFile`] from [`SnapshotOptions`].
    ///
    /// # Arguments
    ///
    /// * `opts` - The [`SnapshotOptions`] to use
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::NonUnicodeHostname`] - If the hostname is not valid unicode
    /// * [`SnapshotFileErrorKind::OutOfRange`] - If the delete time is not in the range of `Local::now()`
    /// * [`SnapshotFileErrorKind::ReadingDescriptionFailed`] - If the description file could not be read
    ///
    /// # Note
    ///
    /// This is the preferred way to create a new [`SnapshotFile`] to be used within [`crate::Repository::backup`].
    ///
    /// [`SnapshotFileErrorKind::NonUnicodeHostname`]: crate::error::SnapshotFileErrorKind::NonUnicodeHostname
    /// [`SnapshotFileErrorKind::OutOfRange`]: crate::error::SnapshotFileErrorKind::OutOfRange
    /// [`SnapshotFileErrorKind::ReadingDescriptionFailed`]: crate::error::SnapshotFileErrorKind::ReadingDescriptionFailed
    pub fn from_options(opts: &SnapshotOptions) -> RusticResult<Self> {
        let hostname = if let Some(host) = &opts.host {
            host.clone()
        } else {
            let hostname = gethostname();
            hostname
                .to_str()
                .ok_or_else(|| SnapshotFileErrorKind::NonUnicodeHostname(hostname.clone()))?
                .to_string()
        };

        let time = opts.time.unwrap_or_else(Local::now);

        let delete = match (opts.delete_never, opts.delete_after) {
            (true, _) => DeleteOption::Never,
            (_, Some(d)) => DeleteOption::After(
                time + Duration::from_std(*d).map_err(SnapshotFileErrorKind::OutOfRange)?,
            ),
            (false, None) => DeleteOption::NotSet,
        };

        let command: String = opts.command.as_ref().map_or_else(
            || {
                std::env::args_os()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            },
            Clone::clone,
        );

        let mut snap = Self {
            time,
            hostname,
            label: opts.label.clone().unwrap_or_default(),
            delete,
            summary: Some(SnapshotSummary {
                command,
                ..Default::default()
            }),
            description: opts.description.clone(),
            ..Default::default()
        };

        // use description from description file if it is given
        if let Some(ref file) = opts.description_from {
            snap.description = Some(
                std::fs::read_to_string(file)
                    .map_err(SnapshotFileErrorKind::ReadingDescriptionFailed)?,
            );
        }

        _ = snap.set_tags(opts.tags.clone());

        Ok(snap)
    }

    /// Create a [`SnapshotFile`] from a given [`Id`] and [`RepoFile`].
    ///
    /// # Arguments
    ///
    /// * `tuple` - A tuple of the [`Id`] and the [`RepoFile`] to use
    fn set_id(tuple: (SnapshotId, Self)) -> Self {
        let (id, mut snap) = tuple;
        snap.id = id;
        _ = snap.original.get_or_insert(id);
        snap
    }

    /// Get a [`SnapshotFile`] from the backend
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `id` - The id of the snapshot
    fn from_backend<B: DecryptReadBackend>(be: &B, id: &SnapshotId) -> RusticResult<Self> {
        Ok(Self::set_id((*id, be.get_file(id)?)))
    }

    /// Get a [`SnapshotFile`] from the backend by (part of the) Id
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `string` - The (part of the) id of the snapshot
    /// * `predicate` - A predicate to filter the snapshots
    /// * `p` - A progress bar to use
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub(crate) fn from_str<B: DecryptReadBackend>(
        be: &B,
        string: &str,
        predicate: impl FnMut(&Self) -> bool + Send + Sync,
        p: &impl Progress,
    ) -> RusticResult<Self> {
        match string {
            "latest" => Self::latest(be, predicate, p),
            _ => Self::from_id(be, string),
        }
    }

    /// Get the latest [`SnapshotFile`] from the backend
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `predicate` - A predicate to filter the snapshots
    /// * `p` - A progress bar to use
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::NoSnapshotsFound`] - If no snapshots are found
    ///
    /// [`SnapshotFileErrorKind::NoSnapshotsFound`]: crate::error::SnapshotFileErrorKind::NoSnapshotsFound
    pub(crate) fn latest<B: DecryptReadBackend>(
        be: &B,
        predicate: impl FnMut(&Self) -> bool + Send + Sync,
        p: &impl Progress,
    ) -> RusticResult<Self> {
        p.set_title("getting latest snapshot...");
        let mut latest: Option<Self> = None;
        let mut pred = predicate;

        for snap in be.stream_all::<Self>(p)? {
            let (id, mut snap) = snap?;
            if !pred(&snap) {
                continue;
            }

            snap.id = id;
            match &latest {
                Some(l) if l.time > snap.time => {}
                _ => {
                    latest = Some(snap);
                }
            }
        }
        p.finish();
        latest.ok_or_else(|| SnapshotFileErrorKind::NoSnapshotsFound.into())
    }

    /// Get a [`SnapshotFile`] from the backend by (part of the) id
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `id` - The (part of the) id of the snapshot
    ///
    /// # Errors
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub(crate) fn from_id<B: DecryptReadBackend>(be: &B, id: &str) -> RusticResult<Self> {
        info!("getting snapshot...");
        let id = be.find_id(FileType::Snapshot, id)?;
        Self::from_backend(be, &SnapshotId::from(id))
    }

    /// Get a list of [`SnapshotFile`]s from the backend by supplying a list of/parts of their Ids
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `ids` - The list of (parts of the) ids of the snapshots
    /// * `p` - A progress bar to use
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub(crate) fn from_ids<B: DecryptReadBackend, T: AsRef<str>>(
        be: &B,
        ids: &[T],
        p: &impl Progress,
    ) -> RusticResult<Vec<Self>> {
        Self::update_from_ids(be, Vec::new(), ids, p)
    }

    /// Update a list of [`SnapshotFile`]s from the backend by supplying a list of/parts of their Ids
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `ids` - The list of (parts of the) ids of the snapshots
    /// * `p` - A progress bar to use
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub(crate) fn update_from_ids<B: DecryptReadBackend, T: AsRef<str>>(
        be: &B,
        current: Vec<Self>,
        ids: &[T],
        p: &impl Progress,
    ) -> RusticResult<Vec<Self>> {
        let ids = be.find_ids(FileType::Snapshot, ids)?;
        Self::fill_missing(be, current, &ids, |_| true, p)
    }

    // helper func
    fn fill_missing<B, F>(
        be: &B,
        current: Vec<Self>,
        ids: &[Id],
        mut filter: F,
        p: &impl Progress,
    ) -> RusticResult<Vec<Self>>
    where
        B: DecryptReadBackend,
        F: FnMut(&Self) -> bool,
    {
        let mut snaps: BTreeMap<_, _> = current.into_iter().map(|snap| (snap.id, snap)).collect();
        let missing_ids: Vec<_> = ids
            .iter()
            .filter(|id| !snaps.contains_key(&SnapshotId::from(**id)))
            .copied()
            .collect();
        for res in be.stream_list::<Self>(&missing_ids, p)? {
            let (id, snap) = res?;
            if filter(&snap) {
                let _ = snaps.insert(id, snap);
            }
        }
        // sort back to original order
        Ok(ids
            .iter()
            .filter_map(|id| snaps.remove_entry(&SnapshotId::from(*id)))
            .map(Self::set_id)
            .collect())
    }

    /// Compare two [`SnapshotFile`]s by criteria from [`SnapshotGroupCriterion`].
    ///
    /// # Arguments
    ///
    /// * `crit` - The criteria to use for comparison
    /// * `other` - The other [`SnapshotFile`] to compare to
    ///
    /// # Returns
    ///
    /// The ordering of the two [`SnapshotFile`]s
    #[must_use]
    pub fn cmp_group(&self, crit: SnapshotGroupCriterion, other: &Self) -> Ordering {
        if crit.hostname {
            self.hostname.cmp(&other.hostname)
        } else {
            Ordering::Equal
        }
        .then_with(|| {
            if crit.label {
                self.label.cmp(&other.label)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            if crit.paths {
                self.paths.cmp(&other.paths)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            if crit.tags {
                self.tags.cmp(&other.tags)
            } else {
                Ordering::Equal
            }
        })
    }

    /// Check if the [`SnapshotFile`] is in the given [`SnapshotGroup`].
    ///
    /// # Arguments
    ///
    /// * `group` - The [`SnapshotGroup`] to check
    #[must_use]
    pub fn has_group(&self, group: &SnapshotGroup) -> bool {
        group
            .hostname
            .as_ref()
            .map_or(true, |val| val == &self.hostname)
            && group.label.as_ref().map_or(true, |val| val == &self.label)
            && group.paths.as_ref().map_or(true, |val| val == &self.paths)
            && group.tags.as_ref().map_or(true, |val| val == &self.tags)
    }

    /// Get [`SnapshotFile`]s which match the filter grouped by the group criterion
    /// from the backend
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `filter` - A filter to filter the snapshots
    /// * `crit` - The criteria to use for grouping
    /// * `p` - A progress bar to use
    pub(crate) fn group_from_backend<B, F>(
        be: &B,
        filter: F,
        crit: SnapshotGroupCriterion,
        p: &impl Progress,
    ) -> RusticResult<Vec<(SnapshotGroup, Vec<Self>)>>
    where
        B: DecryptReadBackend,
        F: FnMut(&Self) -> bool,
    {
        Self::update_group_from_backend(be, Vec::new(), filter, crit, p)
    }

    pub(crate) fn update_group_from_backend<B, F>(
        be: &B,
        current: Vec<(SnapshotGroup, Vec<Self>)>,
        filter: F,
        crit: SnapshotGroupCriterion,
        p: &impl Progress,
    ) -> RusticResult<Vec<(SnapshotGroup, Vec<Self>)>>
    where
        B: DecryptReadBackend,
        F: FnMut(&Self) -> bool,
    {
        let current = current.into_iter().flat_map(|(_, snaps)| snaps).collect();
        let mut snaps = Self::update_from_backend(be, current, filter, p)?;
        snaps.sort_unstable_by(|sn1, sn2| sn1.cmp_group(crit, sn2));

        let mut result = Vec::new();
        for (group, snaps) in &snaps
            .into_iter()
            .chunk_by(|sn| SnapshotGroup::from_snapshot(sn, crit))
        {
            result.push((group, snaps.collect()));
        }

        Ok(result)
    }

    // TODO: add documentation!
    pub(crate) fn all_from_backend<B, F>(
        be: &B,
        filter: F,
        p: &impl Progress,
    ) -> RusticResult<Vec<Self>>
    where
        B: DecryptReadBackend,
        F: FnMut(&Self) -> bool,
    {
        be.stream_all::<Self>(p)?
            .into_iter()
            .map_ok(Self::set_id)
            .filter_ok(filter)
            .try_collect()
    }

    // TODO: add documentation!
    pub(crate) fn update_from_backend<B, F>(
        be: &B,
        current: Vec<Self>,
        filter: F,
        p: &impl Progress,
    ) -> RusticResult<Vec<Self>>
    where
        B: DecryptReadBackend,
        F: FnMut(&Self) -> bool,
    {
        let ids = be
            .list(FileType::Snapshot)
            .map_err(RusticErrorKind::Backend)?;
        Self::fill_missing(be, current, &ids, filter, p)
    }

    /// Add tag lists to snapshot.
    ///
    /// # Arguments
    ///
    /// * `tag_lists` - The tag lists to add
    ///
    /// # Returns
    ///
    /// Returns whether snapshot was changed.
    pub fn add_tags(&mut self, tag_lists: Vec<StringList>) -> bool {
        let old_tags = self.tags.clone();
        self.tags.add_all(tag_lists);

        old_tags != self.tags
    }

    /// Set tag lists to snapshot.
    ///
    /// # Arguments
    ///
    /// * `tag_lists` - The tag lists to set
    ///
    /// # Returns
    ///
    /// Returns whether snapshot was changed.
    pub fn set_tags(&mut self, tag_lists: Vec<StringList>) -> bool {
        let old_tags = std::mem::take(&mut self.tags);
        self.tags.add_all(tag_lists);

        old_tags != self.tags
    }

    /// Remove tag lists from snapshot.
    ///
    /// # Arguments
    ///
    /// * `tag_lists` - The tag lists to remove
    ///
    /// # Returns
    ///
    /// Returns whether snapshot was changed.
    pub fn remove_tags(&mut self, tag_lists: &[StringList]) -> bool {
        let old_tags = self.tags.clone();
        self.tags.remove_all(tag_lists);

        old_tags != self.tags
    }

    /// Returns whether a snapshot must be deleted now
    ///
    /// # Arguments
    ///
    /// * `now` - The current time
    #[must_use]
    pub fn must_delete(&self, now: DateTime<Local>) -> bool {
        matches!(self.delete,DeleteOption::After(time) if time < now)
    }

    /// Returns whether a snapshot must be kept now
    ///
    /// # Arguments
    ///
    /// * `now` - The current time
    #[must_use]
    pub fn must_keep(&self, now: DateTime<Local>) -> bool {
        match self.delete {
            DeleteOption::Never => true,
            DeleteOption::After(time) if time >= now => true,
            _ => false,
        }
    }

    /// Modifies the snapshot setting/adding/removing tag(s) and modifying [`DeleteOption`]s.
    ///
    /// # Arguments
    ///
    /// * `set` - The tags to set
    /// * `add` - The tags to add
    /// * `remove` - The tags to remove
    /// * `delete` - The delete option to set
    ///
    /// # Returns
    ///
    /// `None` if the snapshot was not changed and
    /// `Some(snap)` with a copy of the changed snapshot if it was changed.
    pub fn modify_sn(
        &mut self,
        set: Vec<StringList>,
        add: Vec<StringList>,
        remove: &[StringList],
        delete: &Option<DeleteOption>,
    ) -> Option<Self> {
        let mut changed = false;

        if !set.is_empty() {
            changed |= self.set_tags(set);
        }
        changed |= self.add_tags(add);
        changed |= self.remove_tags(remove);

        if let Some(delete) = delete {
            if &self.delete != delete {
                self.delete = *delete;
                changed = true;
            }
        }

        changed.then_some(self.clone())
    }

    /// Clear ids which are not saved by the copy command (and not compared when checking if snapshots already exist in the copy target)
    ///
    /// # Arguments
    ///
    /// * `sn` - The snapshot to clear the ids from
    #[must_use]
    pub(crate) fn clear_ids(mut sn: Self) -> Self {
        sn.id = SnapshotId::default();
        sn.parent = None;
        sn
    }
}

impl PartialEq<Self> for SnapshotFile {
    fn eq(&self, other: &Self) -> bool {
        self.time.eq(&other.time)
    }
}

impl Eq for SnapshotFile {}

impl PartialOrd for SnapshotFile {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SnapshotFile {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time.cmp(&other.time)
    }
}

/// [`SnapshotGroupCriterion`] determines how to group snapshots.
///
/// `Default` grouping is by hostname, label and paths.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Copy, Setters)]
#[setters(into)]
#[non_exhaustive]
pub struct SnapshotGroupCriterion {
    /// Whether to group by hostnames
    pub hostname: bool,

    /// Whether to group by labels
    pub label: bool,

    /// Whether to group by paths
    pub paths: bool,

    /// Whether to group by tags
    pub tags: bool,
}

impl SnapshotGroupCriterion {
    /// Create a new empty `SnapshotGroupCriterion`
    #[must_use]
    pub fn new() -> Self {
        Self {
            hostname: false,
            label: false,
            paths: false,
            tags: false,
        }
    }
}

impl Default for SnapshotGroupCriterion {
    fn default() -> Self {
        Self {
            hostname: true,
            label: true,
            paths: true,
            tags: false,
        }
    }
}

impl FromStr for SnapshotGroupCriterion {
    type Err = RusticError;
    fn from_str(s: &str) -> RusticResult<Self> {
        let mut crit = Self::new();
        for val in s.split(',') {
            match val {
                "host" => crit.hostname = true,
                "label" => crit.label = true,
                "paths" => crit.paths = true,
                "tags" => crit.tags = true,
                "" => continue,
                v => return Err(SnapshotFileErrorKind::ValueNotAllowed(v.into()).into()),
            }
        }
        Ok(crit)
    }
}

impl Display for SnapshotGroupCriterion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut display = Vec::new();
        if self.hostname {
            display.push("host");
        }
        if self.label {
            display.push("label");
        }
        if self.paths {
            display.push("paths");
        }
        if self.tags {
            display.push("tags");
        }
        write!(f, "{}", display.join(","))?;
        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Serialize, Default, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// [`SnapshotGroup`] specifies the group after a grouping using [`SnapshotGroupCriterion`].
pub struct SnapshotGroup {
    /// Group hostname, if grouped by hostname
    pub hostname: Option<String>,

    /// Group label, if grouped by label
    pub label: Option<String>,

    /// Group paths, if grouped by paths
    pub paths: Option<StringList>,

    /// Group tags, if grouped by tags
    pub tags: Option<StringList>,
}

impl Display for SnapshotGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = Vec::new();

        if let Some(host) = &self.hostname {
            out.push(format!("host [{host}]"));
        }
        if let Some(label) = &self.label {
            out.push(format!("label [{label}]"));
        }
        if let Some(paths) = &self.paths {
            out.push(format!("paths [{paths}]"));
        }
        if let Some(tags) = &self.tags {
            out.push(format!("tags [{tags}]"));
        }

        write!(f, "({})", out.join(", "))?;
        Ok(())
    }
}

impl SnapshotGroup {
    /// Extracts the suitable [`SnapshotGroup`] from a [`SnapshotFile`] using a given [`SnapshotGroupCriterion`].
    ///
    /// # Arguments
    ///
    /// * `sn` - The [`SnapshotFile`] to extract the [`SnapshotGroup`] from
    /// * `crit` - The [`SnapshotGroupCriterion`] to use
    #[must_use]
    pub fn from_snapshot(sn: &SnapshotFile, crit: SnapshotGroupCriterion) -> Self {
        Self {
            hostname: crit.hostname.then(|| sn.hostname.clone()),
            label: crit.label.then(|| sn.label.clone()),
            paths: crit.paths.then(|| sn.paths.clone()),
            tags: crit.tags.then(|| sn.tags.clone()),
        }
    }

    /// Returns whether this is an empty group, i.e. no grouping information is contained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// `StringList` is a rustic-internal list of Strings. It is used within [`SnapshotFile`]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct StringList(pub(crate) BTreeSet<String>);

impl FromStr for StringList {
    type Err = RusticError;
    fn from_str(s: &str) -> RusticResult<Self> {
        Ok(Self(s.split(',').map(ToString::to_string).collect()))
    }
}

impl Display for StringList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().join(","))?;
        Ok(())
    }
}

impl StringList {
    /// Returns whether a [`StringList`] contains a given String.
    ///
    /// # Arguments
    ///
    /// * `s` - The String to check
    #[must_use]
    pub fn contains(&self, s: &str) -> bool {
        self.0.contains(s)
    }

    /// Returns whether a [`StringList`] contains all Strings of another [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `sl` - The [`StringList`] to check
    #[must_use]
    pub fn contains_all(&self, sl: &Self) -> bool {
        sl.0.is_subset(&self.0)
    }

    /// Returns whether a [`StringList`] matches a list of [`StringList`]s,
    /// i.e. whether it contains all Strings of one the given [`StringList`]s.
    ///
    /// # Arguments
    ///
    /// * `sls` - The list of [`StringList`]s to check
    #[must_use]
    pub fn matches(&self, sls: &[Self]) -> bool {
        sls.is_empty() || sls.iter().any(|sl| self.contains_all(sl))
    }

    /// Add a String to a [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `s` - The String to add
    pub fn add(&mut self, s: String) {
        _ = self.0.insert(s);
    }

    /// Add all Strings from another [`StringList`] to this [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `sl` - The [`StringList`] to add
    pub fn add_list(&mut self, mut sl: Self) {
        self.0.append(&mut sl.0);
    }

    /// Add all Strings from all given [`StringList`]s to this [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `string_lists` - The [`StringList`]s to add
    pub fn add_all(&mut self, string_lists: Vec<Self>) {
        for sl in string_lists {
            self.add_list(sl);
        }
    }

    /// Adds the given Paths as Strings to this [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `paths` - The Paths to add
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::NonUnicodePath`] - If a path is not valid unicode
    ///
    /// [`SnapshotFileErrorKind::NonUnicodePath`]: crate::error::SnapshotFileErrorKind::NonUnicodePath
    pub(crate) fn set_paths<T: AsRef<Path>>(&mut self, paths: &[T]) -> RusticResult<()> {
        self.0 = paths
            .iter()
            .map(|p| {
                Ok(p.as_ref()
                    .to_str()
                    .ok_or_else(|| SnapshotFileErrorKind::NonUnicodePath(p.as_ref().to_path_buf()))?
                    .to_string())
            })
            .collect::<RusticResult<BTreeSet<_>>>()?;
        Ok(())
    }

    /// Remove all Strings from all given [`StringList`]s from this [`StringList`].
    ///
    /// # Arguments
    ///
    /// * `string_lists` - The [`StringList`]s to remove
    pub fn remove_all(&mut self, string_lists: &[Self]) {
        for sl in string_lists {
            self.0 = &self.0 - &sl.0;
        }
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    #[deprecated(note = "StringLists are now automatically sorted")]
    /// Sort the Strings in the [`StringList`]
    pub fn sort(&mut self) {}

    /// Format this [`StringList`] using newlines
    #[must_use]
    pub fn formatln(&self) -> String {
        self.0.iter().join("\n")
    }

    /// Turn this [`StringList`] into an Iterator
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }
}

impl<'str> IntoIterator for &'str StringList {
    type Item = &'str String;
    type IntoIter = std::collections::btree_set::Iter<'str, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// `PathList` is a rustic-internal list of `PathBuf`s. It is used in the [`crate::Repository::backup`] command.
#[derive(Default, Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct PathList(Vec<PathBuf>);

impl Display for PathList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .map(|p| p.to_string_lossy())
            .format(",")
            .fmt(f)
    }
}

impl<T: Into<PathBuf>> FromIterator<T> for PathList {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(iter.into_iter().map(T::into).collect())
    }
}

impl PathList {
    /// Create a `PathList` from a String containing a single path
    /// Note: for multiple paths, use `PathList::from_iter`.
    ///
    /// # Arguments
    ///
    /// * `source` - The String to parse
    ///
    /// # Errors
    /// no errors can occur here
    pub fn from_string(source: &str) -> RusticResult<Self> {
        Ok(Self(vec![source.into()]))
    }

    /// Number of paths in the `PathList`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the `PathList` is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    /// Clone the internal `Vec<PathBuf>`.
    #[must_use]
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.0.clone()
    }

    /// Sanitize paths: Parse dots, absolutize if needed and merge paths.
    ///
    /// # Errors
    ///
    /// * [`SnapshotFileErrorKind::RemovingDotsFromPathFailed`] - If removing dots from path failed
    /// * [`SnapshotFileErrorKind::CanonicalizingPathFailed`] - If canonicalizing path failed
    ///
    /// [`SnapshotFileErrorKind::RemovingDotsFromPathFailed`]: crate::error::SnapshotFileErrorKind::RemovingDotsFromPathFailed
    /// [`SnapshotFileErrorKind::CanonicalizingPathFailed`]: crate::error::SnapshotFileErrorKind::CanonicalizingPathFailed
    pub fn sanitize(mut self) -> RusticResult<Self> {
        for path in &mut self.0 {
            *path = sanitize_dot(path)?;
        }
        if self.0.iter().any(|p| p.is_absolute()) {
            self.0 = self
                .0
                .into_iter()
                .map(|p| canonicalize(p).map_err(SnapshotFileErrorKind::CanonicalizingPathFailed))
                .collect::<Result<_, _>>()?;
        }
        Ok(self.merge())
    }

    /// Sort paths and filters out subpaths of already existing paths.
    #[must_use]
    pub fn merge(self) -> Self {
        let mut paths = self.0;
        // sort paths
        paths.sort_unstable();

        let mut root_path = None;

        // filter out subpaths
        paths.retain(|path| match &root_path {
            Some(root_path) if path.starts_with(root_path) => false,
            _ => {
                root_path = Some(path.clone());
                true
            }
        });

        Self(paths)
    }
}

// helper function to sanitize paths containing dots
fn sanitize_dot(path: &Path) -> RusticResult<PathBuf> {
    if path == Path::new(".") || path == Path::new("./") {
        return Ok(PathBuf::from("."));
    }

    let path = if path.starts_with("./") {
        path.strip_prefix("./").unwrap()
    } else {
        path
    };

    let path = path
        .parse_dot()
        .map_err(SnapshotFileErrorKind::RemovingDotsFromPathFailed)?
        .to_path_buf();

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rstest::rstest;

    #[rstest]
    #[case(".", ".")]
    #[case("./", ".")]
    #[case("test", "test")]
    #[case("test/", "test")]
    #[case("./test", "test")]
    #[case("./test/", "test")]
    fn sanitize_dot_cases(#[case] input: &str, #[case] expected: &str) {
        let path = Path::new(input);
        let expected = PathBuf::from(expected);

        assert_eq!(expected, sanitize_dot(path).unwrap());
    }

    #[rstest]
    #[case("abc", vec!["abc".to_string()])]
    #[case("abc,def", vec!["abc".to_string(), "def".to_string()])]
    #[case("abc,abc", vec!["abc".to_string()])]
    fn test_set_tags(#[case] tag: &str, #[case] expected: Vec<String>) -> Result<()> {
        let mut snap = SnapshotFile::from_options(&SnapshotOptions::default())?;
        let tags = StringList::from_str(tag)?;
        let expected = StringList(expected.into_iter().collect());
        assert!(snap.set_tags(vec![tags]));
        assert_eq!(snap.tags, expected);
        Ok(())
    }

    #[test]
    fn test_add_tags() -> Result<()> {
        let tags = vec![StringList::from_str("abc")?];
        let mut snap = SnapshotFile::from_options(&SnapshotOptions::default().tags(tags))?;
        let tags = StringList::from_str("def,abc")?;
        assert!(snap.add_tags(vec![tags]));
        let expected = StringList::from_str("abc,def")?;
        assert_eq!(snap.tags, expected);
        Ok(())
    }

    #[rstest]
    #[case("host,label,paths", true, true, true, false)]
    #[case("host", true, false, false, false)]
    #[case("label,host", true, true, false, false)]
    #[case("tags", false, false, false, true)]
    #[case("paths,label", false, true, true, false)]
    fn snapshot_group_criterion_fromstr(
        #[case] crit: SnapshotGroupCriterion,
        #[case] is_host: bool,
        #[case] is_label: bool,
        #[case] is_path: bool,
        #[case] is_tags: bool,
    ) {
        assert_eq!(crit.hostname, is_host);
        assert_eq!(crit.label, is_label);
        assert_eq!(crit.paths, is_path);
        assert_eq!(crit.tags, is_tags);
    }

    #[rstest]
    #[case(vec![], "")]
    #[case(vec!["test"], "test")]
    #[case(vec!["test", "test", "test"], "test,test,test")]
    fn test_display_path_list_passes(#[case] input: Vec<&str>, #[case] expected: &str) {
        let path_list = PathList::from_iter(input);
        let result = path_list.to_string();
        assert_eq!(expected, &result);
    }
}
