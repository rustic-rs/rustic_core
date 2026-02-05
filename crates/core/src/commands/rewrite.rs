use std::path::PathBuf;

use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{
    ErrorKind, IndexedFull, Open, Repository, RusticError, RusticResult, StringList,
    blob::tree::{
        modify::ModifierChange,
        rewrite::{RewriteTreesOptions, Rewriter},
    },
    repofile::{SnapshotFile, SnapshotModification},
};

/// Options for rewrite
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Debug, Default, Setters, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
pub struct RewriteOptions {
    /// remove original snapshots
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub forget: bool,

    /// Tags to add to rewritten snapshots [default: "rewrite" if original snapshots are not removed]
    #[cfg_attr(feature = "clap", clap(long, value_name = "TAG[,TAG,..]"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub tags_rewritten: Option<StringList>,

    /// Snapshot modifications
    #[cfg_attr(feature = "clap", clap(flatten))]
    pub modification: SnapshotModification,

    /// Dry-run: Don't save any modification
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub dry_run: bool,
}

pub(crate) fn rewrite_snapshots_and_trees<S: IndexedFull>(
    repo: &Repository<S>,
    snapshots: Vec<SnapshotFile>,
    opts: &RewriteOptions,
    tree_opts: &RewriteTreesOptions,
) -> RusticResult<Vec<SnapshotFile>> {
    let config_file = repo.config();
    if opts.forget && config_file.append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Removing snapshots is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }
    let mut rewriter = Rewriter::new(
        repo.dbe(),
        repo.index(),
        repo.config(),
        tree_opts,
        opts.dry_run,
    )?;

    let snapshots: Vec<_> = snapshots
        .into_iter()
        .map(|mut sn| {
            #[allow(clippy::useless_let_if_seq)] // false positive lint
            let mut changed = sn.modify(&opts.modification)?;

            if let ModifierChange::Changed(new_tree) =
                rewriter.rewrite_tree(PathBuf::new(), sn.tree)?
            {
                sn.tree = new_tree;
                changed = true;
            }

            if let Some(summary) = rewriter.summary(&sn.tree) {
                let mut snap_summary = sn.summary.clone().unwrap_or_default();
                snap_summary.total_files_processed = summary.files;
                snap_summary.total_bytes_processed = summary.size;
                snap_summary.total_dirs_processed = summary.dirs;
                changed |= sn.summary.is_none_or(|sum| sum != snap_summary);
                sn.summary = Some(snap_summary);
            }

            Ok(changed.then_some(sn))
        })
        .filter_map(Result::transpose)
        .collect::<RusticResult<_>>()?;

    rewriter.finalize()?;

    process_snapshots(repo, snapshots, opts)
}

pub(crate) fn rewrite_snapshots<S: Open>(
    repo: &Repository<S>,
    snapshots: Vec<SnapshotFile>,
    opts: &RewriteOptions,
) -> RusticResult<Vec<SnapshotFile>> {
    let config_file = repo.config();
    if opts.forget && config_file.append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Removing snapshots is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }
    let snapshots: Vec<_> = snapshots
        .into_iter()
        .map(|mut sn| Ok(sn.modify(&opts.modification)?.then_some(sn)))
        .filter_map(Result::transpose)
        .collect::<RusticResult<_>>()?;

    process_snapshots(repo, snapshots, opts)
}

fn process_snapshots<S: Open>(
    repo: &Repository<S>,
    mut snapshots: Vec<SnapshotFile>,
    opts: &RewriteOptions,
) -> RusticResult<Vec<SnapshotFile>> {
    if !snapshots.is_empty() && !opts.dry_run {
        match (&opts.tags_rewritten, opts.forget) {
            (Some(tags), _) => snapshots
                .iter_mut()
                .for_each(|sn| _ = sn.add_tags(vec![tags.clone()])),
            (None, false) => snapshots
                .iter_mut()
                .for_each(|sn| sn.tags.add("rewrite".to_string())),
            (None, true) => {}
        }

        repo.save_snapshots(snapshots.clone())?;
        if opts.forget {
            let old_snap_ids: Vec<_> = snapshots.iter().map(|sn| sn.id).collect();
            repo.delete_snapshots(&old_snap_ids)?;
        }
    }

    Ok(snapshots)
}
