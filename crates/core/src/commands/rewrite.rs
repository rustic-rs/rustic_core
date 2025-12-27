use std::path::PathBuf;

use crate::{
    ErrorKind, Excludes, IndexedFull, Open, ProgressBars, Repository, RusticError, RusticResult,
    blob::tree::{modify::ModifierChange, rewrite::Rewriter},
    repofile::{SnapshotFile, SnapshotModification},
    repository::IndexedTree,
};

pub(crate) fn rewrite_snapshots_with_excludes<P: ProgressBars, S: IndexedFull>(
    repo: &Repository<P, S>,
    snapshots: Vec<SnapshotFile>,
    modification: &SnapshotModification,
    excludes: &Excludes,
    dry_run: bool,
    delete: bool,
) -> RusticResult<Vec<SnapshotFile>> {
    let config_file = repo.config();
    if delete && config_file.append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Removing snapshots is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }
    let mut rewriter = Rewriter::new(repo.dbe(), repo.index(), repo.config(), dry_run, excludes)?;

    let snapshots: Vec<_> = snapshots
        .into_iter()
        .map(|mut sn| {
            #[allow(clippy::useless_let_if_seq)] // false positive lint
            let mut changed = sn.modify(modification)?;

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

    process_snapshots(repo, snapshots, dry_run, delete)
}

pub(crate) fn rewrite_snapshots<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
    snapshots: Vec<SnapshotFile>,
    modification: &SnapshotModification,
    dry_run: bool,
    delete: bool,
) -> RusticResult<Vec<SnapshotFile>> {
    let config_file = repo.config();
    if delete && config_file.append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Removing snapshots is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }
    let snapshots: Vec<_> = snapshots
        .into_iter()
        .map(|mut sn| Ok(sn.modify(modification)?.then_some(sn)))
        .filter_map(Result::transpose)
        .collect::<RusticResult<_>>()?;

    process_snapshots(repo, snapshots, dry_run, delete)
}

fn process_snapshots<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
    snapshots: Vec<SnapshotFile>,
    dry_run: bool,
    delete: bool,
) -> RusticResult<Vec<SnapshotFile>> {
    if !snapshots.is_empty() && !dry_run {
        repo.save_snapshots(snapshots.clone())?;
        if delete {
            let old_snap_ids: Vec<_> = snapshots.iter().map(|sn| sn.id).collect();
            repo.delete_snapshots(&old_snap_ids)?;
        }
    }

    Ok(snapshots)
}
