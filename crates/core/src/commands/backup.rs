//! `backup` subcommand
use derive_setters::Setters;
use itertools::Itertools;
use log::info;

use std::path::PathBuf;

use path_dedot::ParseDot;
use serde_derive::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    CommandInput, Excludes,
    archiver::{Archiver, parent::Parent},
    backend::{
        childstdout::ChildStdoutSource,
        dry_run::DryRunBackend,
        ignore::{LocalSource, LocalSourceFilterOptions, LocalSourceSaveOptions},
        stdin::StdinSource,
    },
    error::{ErrorKind, RusticError, RusticResult},
    repofile::{
        PathList, SnapshotFile,
        snapshotfile::{SnapshotGroup, SnapshotGroupCriterion, SnapshotId},
    },
    repository::{IndexedIds, IndexedTree, Repository},
};

#[cfg(feature = "clap")]
use clap::ValueHint;

/// `backup` subcommand
#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Default, Debug, Deserialize, Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[allow(clippy::struct_excessive_bools)]
#[non_exhaustive]
/// Options how the backup command uses a parent snapshot.
pub struct ParentOptions {
    /// Group snapshots by any combination of host,label,paths,tags to find a suitable parent (default: host,label,paths)
    #[cfg_attr(feature = "clap", clap(long, short = 'g', value_name = "CRITERION",))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub group_by: Option<SnapshotGroupCriterion>,

    /// Snapshot to use as parent (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long = "parent", value_name = "SNAPSHOT", conflicts_with = "force")
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::append))]
    pub parents: Vec<String>,

    /// Skip writing of snapshot if nothing changed w.r.t. the parent snapshot.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub skip_if_unchanged: bool,

    /// Use no parent, read all files
    #[cfg_attr(feature = "clap", clap(long, short))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub force: bool,

    /// Ignore ctime changes when checking for modified files
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "force"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub ignore_ctime: bool,

    /// Ignore inode number changes when checking for modified files
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "force"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub ignore_inode: bool,
}

impl ParentOptions {
    /// Get parent snapshot.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bars.
    /// * `S` - The type of the indexed tree.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to use
    /// * `snap` - The snapshot to use
    /// * `backup_stdin` - Whether the backup is from stdin
    ///
    /// # Returns
    ///
    /// The parent snapshot id and the parent object or `None` if no parent is used.
    pub(crate) fn get_parent<S: IndexedTree>(
        &self,
        repo: &Repository<S>,
        snap: &SnapshotFile,
        backup_stdin: bool,
    ) -> (Vec<SnapshotId>, Parent) {
        let group = SnapshotGroup::from_snapshot(snap, self.group_by.unwrap_or_default());
        let parent = if backup_stdin || self.force {
            Vec::new()
        } else if self.parents.is_empty() {
            // get suitable snapshot group from snapshot and opts.group_by. This is used to filter snapshots for the parent detection
            SnapshotFile::latest(
                repo.dbe(),
                |snap| snap.has_group(&group),
                &repo.progress_counter(""),
            )
            .ok()
            .into_iter()
            .collect()
        } else {
            SnapshotFile::from_strs(
                repo.dbe(),
                &self.parents,
                |snap| snap.has_group(&group),
                &repo.progress_counter(""),
            )
            .unwrap_or_default()
        };

        let (parent_trees, parent_ids): (Vec<_>, _) = parent
            .into_iter()
            .map(|parent| (parent.tree, parent.id))
            .unzip();

        (
            parent_ids,
            Parent::new(
                repo.dbe(),
                repo.index(),
                parent_trees,
                self.ignore_ctime,
                self.ignore_inode,
            ),
        )
    }
}

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Default, Debug, Deserialize, Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `backup` command.
pub struct BackupOptions {
    /// Set filename to be used when backing up from stdin
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "FILENAME", default_value = "stdin", value_hint = ValueHint::FilePath)
    )]
    #[cfg_attr(feature = "merge", merge(skip))]
    pub stdin_filename: String,

    /// Call the given command and use its output as stdin
    #[cfg_attr(feature = "clap", clap(long, value_name = "COMMAND"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub stdin_command: Option<CommandInput>,

    /// Manually set backup path in snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "PATH", value_hint = ValueHint::DirPath))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub as_path: Option<PathBuf>,

    /// Don't scan the backup source for its size - this disables ETA estimation for backup.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub no_scan: bool,

    /// Dry-run mode: Don't write any data or snapshot
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub dry_run: bool,

    #[cfg_attr(feature = "clap", clap(flatten))]
    #[serde(flatten)]
    /// Options how to use a parent snapshot
    pub parent_opts: ParentOptions,

    #[cfg_attr(feature = "clap", clap(flatten))]
    #[serde(flatten)]
    /// Options how to save entries from a local source
    pub ignore_save_opts: LocalSourceSaveOptions,

    #[cfg_attr(feature = "clap", clap(flatten))]
    #[serde(flatten)]
    /// excludes
    pub excludes: Excludes,

    #[cfg_attr(feature = "clap", clap(flatten))]
    #[serde(flatten)]
    /// Options how to filter from a local source
    pub ignore_filter_opts: LocalSourceFilterOptions,
}

/// Backup data, create a snapshot.
///
/// # Type Parameters
///
/// * `P` - The type of the progress bars.
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to use
/// * `opts` - The backup options
/// * `source` - The source to backup
/// * `snap` - The snapshot to backup
///
/// # Errors
///
/// * If sending the message to the raw packer fails.
/// * If converting the data length to u64 fails
/// * If sending the message to the raw packer fails.
/// * If the index file could not be serialized.
/// * If the time is not in the range of `Local::now()`
///
/// # Returns
///
/// The snapshot pointing to the backup'ed data.
#[allow(clippy::too_many_lines)]
pub(crate) fn backup<S: IndexedIds>(
    repo: &Repository<S>,
    opts: &BackupOptions,
    source: &PathList,
    mut snap: SnapshotFile,
) -> RusticResult<SnapshotFile> {
    let index = repo.index();

    let backup_stdin = *source == PathList::from_string("-")?;
    let backup_path = if backup_stdin {
        vec![PathBuf::from(&opts.stdin_filename)]
    } else {
        source.paths()
    };

    let as_path = opts
        .as_path
        .as_ref()
        .map(|p| -> RusticResult<_> {
            Ok(p.parse_dot()
                .map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Failed to parse dotted path `{path}`",
                        err,
                    )
                    .attach_context("path", p.display().to_string())
                })?
                .to_path_buf())
        })
        .transpose()?;

    let paths = match &as_path {
        Some(p) => std::slice::from_ref(p),
        None => &backup_path,
    };

    snap.paths.set_paths(paths).map_err(|err| {
        RusticError::with_source(
            ErrorKind::Internal,
            "Failed to set paths `{paths}` in snapshot.",
            err,
        )
        .attach_context(
            "paths",
            backup_path
                .iter()
                .map(|p| p.display().to_string())
                .join(","),
        )
    })?;

    let (parent_ids, parent) = opts.parent_opts.get_parent(repo, &snap, backup_stdin);
    if parent_ids.is_empty() {
        info!("using no parent");
    } else {
        info!("using parents {}", parent_ids.iter().join(", "));
        snap.parent = Some(parent_ids[0]);
        snap.parents = parent_ids;
    }

    let be = DryRunBackend::new(repo.dbe().clone(), opts.dry_run);
    info!("starting to backup {source} ...");
    let archiver = Archiver::new(be, index, repo.config(), parent, snap)?;
    let p = repo.progress_bytes("backing up...");

    let snap = if backup_stdin {
        let path = &backup_path[0];
        if let Some(command) = &opts.stdin_command {
            let src = ChildStdoutSource::new(command, path.clone())?;
            let res = archiver.archive(
                &src,
                path,
                as_path.as_ref(),
                opts.parent_opts.skip_if_unchanged,
                opts.no_scan,
                &p,
            )?;
            src.finish()?;
            res
        } else {
            let src = StdinSource::new(path.clone());
            archiver.archive(
                &src,
                path,
                as_path.as_ref(),
                opts.parent_opts.skip_if_unchanged,
                opts.no_scan,
                &p,
            )?
        }
    } else {
        let src = LocalSource::new(
            opts.ignore_save_opts,
            &opts.excludes,
            &opts.ignore_filter_opts,
            &backup_path,
        )?;
        archiver.archive(
            &src,
            &backup_path[0],
            as_path.as_ref(),
            opts.parent_opts.skip_if_unchanged,
            opts.no_scan,
            &p,
        )?
    };

    Ok(snap)
}
