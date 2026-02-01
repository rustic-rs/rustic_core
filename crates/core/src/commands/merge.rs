//! `merge` subcommand

use std::cmp::Ordering;

use jiff::Zoned;

use crate::{
    backend::{decrypt::DecryptWriteBackend, node::Node},
    blob::{
        BlobId, BlobType,
        packer::{PackSizer, Packer},
        tree::{self, Tree, TreeId},
    },
    error::{ErrorKind, RusticError, RusticResult},
    index::{ReadIndex, indexer::Indexer},
    repofile::{PathList, SnapshotFile, SnapshotSummary},
    repository::{IndexedTree, Repository},
};

/// Merges the given snapshots into a new snapshot.
///
/// # Arguments
///
/// * `repo` - The repository to merge into
/// * `snapshots` - The snapshots to merge
/// * `cmp` - The comparison function for the trees
/// * `snap` - The snapshot to merge into
///
/// # Returns
///
/// The merged snapshot
pub(crate) fn merge_snapshots<S: IndexedTree>(
    repo: &Repository<S>,
    snapshots: &[SnapshotFile],
    cmp: &impl Fn(&Node, &Node) -> Ordering,
    mut snap: SnapshotFile,
) -> RusticResult<SnapshotFile> {
    let now = Zoned::now();

    let paths = snapshots
        .iter()
        .flat_map(|snap| snap.paths.iter())
        .collect::<PathList>()
        .merge();

    snap.paths.set_paths(&paths.paths()).map_err(|err| {
        RusticError::with_source(
            ErrorKind::Internal,
            "Failed to set paths `{paths}` in snapshot.",
            err,
        )
        .attach_context("paths", paths.to_string())
    })?;

    // set snapshot time to time of latest snapshot to be merged
    snap.time = snapshots
        .iter()
        .max_by(|sn1, sn2| sn1.time.cmp(&sn2.time))
        .map_or_else(|| now.clone(), |sn| sn.time.clone());

    let mut summary = snap.summary.take().unwrap_or_default();
    summary.backup_start = now.clone();

    let trees: Vec<TreeId> = snapshots.iter().map(|sn| sn.tree).collect();
    snap.tree = merge_trees(repo, &trees, cmp, &mut summary)?;

    summary.finalize(&now);
    snap.summary = Some(summary);

    snap.id = repo.dbe().save_file(&snap)?.into();
    Ok(snap)
}

/// Merges the given trees into a new tree.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to merge into
/// * `trees` - The trees to merge
/// * `cmp` - The comparison function for the trees
/// * `summary` - The summary to update
///
/// # Errors
///
/// * If the size of the tree is too large
///
/// # Returns
///
/// The merged tree
pub(crate) fn merge_trees<S: IndexedTree>(
    repo: &Repository<S>,
    trees: &[TreeId],
    cmp: &impl Fn(&Node, &Node) -> Ordering,
    summary: &mut SnapshotSummary,
) -> RusticResult<TreeId> {
    let be = repo.dbe();
    let index = repo.index();
    let indexer = Indexer::new(repo.dbe().clone()).into_shared();
    let pack_sizer = PackSizer::from_config(
        repo.config(),
        BlobType::Tree,
        index.total_size(BlobType::Tree),
    );
    let packer = Packer::new(
        repo.dbe().clone(),
        BlobType::Tree,
        indexer.clone(),
        pack_sizer,
    )?;

    let save = |tree: Tree| -> RusticResult<_> {
        let (chunk, new_id) = tree.serialize().map_err(|err| {
            RusticError::with_source(ErrorKind::Internal, "Failed to serialize tree.", err)
        })?;

        let size = u64::try_from(chunk.len()).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to convert chunk length `{length}` to u64.",
                err,
            )
            .attach_context("length", chunk.len().to_string())
        })?;

        if !index.has_tree(&new_id) {
            packer.add(chunk.into(), BlobId::from(*new_id))?;
        }

        Ok((new_id, size))
    };

    let p = repo.progress_spinner("merging snapshots...");
    let tree_merged = tree::merge_trees(be, index, trees, cmp, &save, summary)?;
    let stats = packer.finalize()?;
    indexer.write().unwrap().finalize()?;
    p.finish();

    stats.apply(summary, BlobType::Tree);

    Ok(tree_merged)
}
