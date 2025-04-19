//! `backup` subcommand
use derive_setters::Setters;
use log::info;

use std::path::PathBuf;

use path_dedot::ParseDot;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};

use crate::{
    archiver::{parent::Parent, Archiver},
    backend::{
        childstdout::ChildStdoutSource,
        dry_run::DryRunBackend,
        ignore::{LocalSource, LocalSourceFilterOptions, LocalSourceSaveOptions},
        stdin::StdinSource,
    },
    error::{ErrorKind, RusticError, RusticResult},
    progress::ProgressBars,
    repofile::{
        snapshotfile::{SnapshotGroup, SnapshotGroupCriterion, SnapshotId},
        PathList, SnapshotFile,
    },
    repository::{IndexedIds, IndexedTree, Repository},
    CommandInput,
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

    /// Snapshot to use as parent
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "SNAPSHOT", conflicts_with = "force",)
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub parent: Option<String>,

    /// Skip writing of snapshot if nothing changed w.r.t. the parent snapshot.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub skip_if_unchanged: bool,

    /// Use no parent, read all files
    #[cfg_attr(feature = "clap", clap(long, short, conflicts_with = "parent",))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub force: bool,

    /// Ignore ctime changes when checking for modified files
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "force",))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub ignore_ctime: bool,

    /// Ignore inode number changes when checking for modified files
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "force",))]
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
    pub(crate) fn get_parent<P: ProgressBars, S: IndexedTree>(
        &self,
        repo: &Repository<P, S>,
        snap: &SnapshotFile,
        backup_stdin: bool,
    ) -> (Option<SnapshotId>, Parent) {
        let parent = match (backup_stdin, self.force, &self.parent) {
            (true, _, _) | (false, true, _) => None,
            (false, false, None) => {
                // get suitable snapshot group from snapshot and opts.group_by. This is used to filter snapshots for the parent detection
                let group = SnapshotGroup::from_snapshot(snap, self.group_by.unwrap_or_default());
                SnapshotFile::latest(
                    repo.dbe(),
                    |snap| snap.has_group(&group),
                    &repo.pb.progress_counter(""),
                )
                .ok()
            }
            (false, false, Some(parent)) => SnapshotFile::from_id(repo.dbe(), parent).ok(),
        };

        let (parent_tree, parent_id) = parent.map(|parent| (parent.tree, parent.id)).unzip();

        (
            parent_id,
            Parent::new(
                repo.dbe(),
                repo.index(),
                parent_tree,
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
pub(crate) fn backup<P: ProgressBars, S: IndexedIds>(
    repo: &Repository<P, S>,
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

    match &as_path {
        Some(p) => snap.paths.set_paths(&[p.clone()]).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to set paths `{paths}` in snapshot.",
                err,
            )
            .attach_context("paths", p.display().to_string())
        })?,
        None => snap.paths.set_paths(&backup_path).map_err(|err| {
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
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })?,
    }

    let (parent_id, parent) = opts.parent_opts.get_parent(repo, &snap, backup_stdin);
    match parent_id {
        Some(id) => {
            info!("using parent {id}");
            snap.parent = Some(id);
        }
        None => {
            info!("using no parent");
        }
    }

    let be = DryRunBackend::new(repo.dbe().clone(), opts.dry_run);
    info!("starting to backup {source} ...");
    let archiver = Archiver::new(be, index, repo.config(), parent, snap)?;
    let p = repo.pb.progress_bytes("backing up...");

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
