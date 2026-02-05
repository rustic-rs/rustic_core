use std::collections::BTreeSet;

use itertools::Itertools;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};

use crate::{
    DataId, Progress, TreeId,
    backend::{
        decrypt::{DecryptFullBackend, DecryptWriteBackend},
        node::NodeType,
    },
    blob::{
        BlobType,
        packer::{BlobCopier, CopyPackBlobs, PackSizer},
        tree::TreeStreamerOnce,
    },
    error::RusticResult,
    index::{ReadIndex, indexer::Indexer},
    repofile::SnapshotFile,
    repository::{IndexedFull, IndexedIds, IndexedTree, Open, Repository},
};

/// This struct enhances `[SnapshotFile]` with the attribute `relevant`
/// which indicates if the snapshot is relevant for copying.
#[derive(Debug, PartialEq, Eq)]
pub struct CopySnapshot {
    /// The snapshot
    pub sn: SnapshotFile,
    /// Whether it is relevant
    pub relevant: bool,
}

/// Copy the given snapshots to the destination repository.
///
/// # Type Parameters
///
/// * `Q` - The progress bar type.
/// * `R` - The type of the indexed tree.
/// * `P` - The progress bar type.
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to copy from
/// * `repo_dest` - The repository to copy to
/// * `snapshots` - The snapshots to copy
///
/// # Errors
///
// TODO: Document errors
pub(crate) fn copy<'a, R: IndexedFull, S: IndexedIds>(
    repo: &Repository<R>,
    repo_dest: &Repository<S>,
    snapshots: impl IntoIterator<Item = &'a SnapshotFile>,
) -> RusticResult<()> {
    let be_dest = repo_dest.dbe();

    let (snap_trees, snaps): (Vec<_>, Vec<_>) = snapshots
        .into_iter()
        .cloned()
        .map(|sn| (sn.tree, SnapshotFile::clear_ids(sn)))
        .unzip();

    let be = repo.dbe();
    let index = repo.index();
    let index_dest = repo_dest.index();

    let filter_tree = |id: &TreeId| !index_dest.has_tree(id);
    let filter_data = |id: &DataId| !index_dest.has_data(id);
    let mut tree_ids: BTreeSet<_> = snap_trees.iter().copied().filter(filter_tree).collect();
    let mut data_ids = BTreeSet::new();

    let p = repo_dest.progress_counter("finding needed blobs...");

    let mut tree_streamer = TreeStreamerOnce::new(be, index, snap_trees, p)?;
    while let Some(item) = tree_streamer.next().transpose()? {
        let (_, tree) = item;
        for node in tree.nodes {
            match node.node_type {
                NodeType::File => {
                    data_ids.extend(node.content.into_iter().flatten().filter(filter_data));
                }
                NodeType::Dir => {
                    tree_ids.extend(node.subtree.into_iter().filter(filter_tree));
                }
                _ => {} // nothing to do
            }
        }
    }

    let indexer = Indexer::new(be_dest.clone()).into_shared();

    let p = repo_dest.progress_bytes("copying data blobs...");
    let pack_sizer = PackSizer::from_config(
        repo_dest.config(),
        BlobType::Data,
        repo_dest.index().total_size(BlobType::Data),
    );
    let data_repacker = BlobCopier::new(
        be.clone(),
        be_dest.clone(),
        BlobType::Data,
        indexer.clone(),
        pack_sizer,
    )?;
    let data_blobs: Vec<_> = data_ids
        .into_iter()
        .filter_map(|id| {
            index
                .get_data(&id)
                .map(|entry| CopyPackBlobs::from_index_entry(entry, id.into()))
        })
        .collect();

    copy_blobs(data_blobs, data_repacker, p)?;

    let p = repo_dest.progress_bytes("copying tree blobs...");
    let pack_sizer = PackSizer::from_config(
        repo_dest.config(),
        BlobType::Tree,
        repo_dest.index().total_size(BlobType::Tree),
    );
    let tree_repacker = BlobCopier::new(
        be.clone(),
        be_dest.clone(),
        BlobType::Tree,
        indexer.clone(),
        pack_sizer,
    )?;

    let trees: Vec<_> = tree_ids
        .into_iter()
        .filter_map(|id| {
            index
                .get_tree(&id)
                .map(|entry| CopyPackBlobs::from_index_entry(entry, id.into()))
        })
        .collect();

    copy_blobs(trees, tree_repacker, p)?;

    indexer.write().unwrap().finalize()?;

    let p = repo_dest.progress_counter("saving snapshots...");
    be_dest.save_list(snaps.iter(), p)?;
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn copy_blobs<BE: DecryptFullBackend>(
    mut blobs: Vec<CopyPackBlobs>,
    copier: BlobCopier<BE>,
    p: Progress,
) -> RusticResult<()> {
    blobs.sort_unstable();
    let blobs: Vec<_> = blobs
        .into_iter()
        .coalesce(CopyPackBlobs::coalesce)
        .collect();

    let length = blobs
        .iter()
        .map(|blob| u64::from(blob.locations.length()))
        .sum();
    p.set_length(length);

    blobs
        .into_par_iter()
        .try_for_each(|blobs| -> RusticResult<_> { copier.copy(blobs, &p) })?;
    _ = copier.finalize()?;
    p.finish();
    Ok(())
}

/// Filter out relevant snapshots from the given list of snapshots.
///
/// # Type Parameters
///
/// * `F` - The type of the filter.
/// * `P` - The progress bar type.
/// * `S` - The state of the repository.
///
/// # Arguments
///
/// * `snaps` - The snapshots to filter
/// * `dest_repo` - The destination repository
/// * `filter` - The filter to apply to the snapshots
///
/// # Errors
///
// TODO: Document errors
///
/// # Returns
///
/// A list of snapshots with the attribute `relevant` set to `true` if the snapshot is relevant for copying.
pub(crate) fn relevant_snapshots<F, S: Open>(
    snaps: &[SnapshotFile],
    dest_repo: &Repository<S>,
    filter: F,
) -> RusticResult<Vec<CopySnapshot>>
where
    F: FnMut(&SnapshotFile) -> bool,
{
    let p = dest_repo.progress_counter("finding relevant snapshots...");
    // save snapshots in destination in BTreeSet, as we want to efficiently search within to filter out already existing snapshots before copying.
    let snapshots_dest: BTreeSet<_> =
        SnapshotFile::iter_all_from_backend(dest_repo.dbe(), filter, &p)?.collect();

    let relevant = snaps
        .iter()
        .cloned()
        .map(|sn| CopySnapshot {
            relevant: !snapshots_dest.contains(&sn),
            sn,
        })
        .collect();

    Ok(relevant)
}
