//! `snapshot` subcommand

use itertools::Itertools;

use crate::{
    Progress,
    error::RusticResult,
    progress::ProgressBars,
    repofile::{
        SnapshotFile,
        snapshotfile::{SnapshotGroup, SnapshotGroupCriterion},
    },
    repository::{Open, Repository},
};

/// Get the snapshots from the repository.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to get the snapshots from.
/// * `ids` - The ids of the snapshots to get.
///   * each `id` can use an actual (short) id "01a2b3c4" or "latest" or "latest~N"
/// * `group_by` - The criterion to group the snapshots by.
/// * `filter` - The filter to apply to the snapshots.
///
/// # Returns
///
/// The snapshots grouped by the given criterion.
pub(crate) fn get_snapshot_group<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
    ids: &[String],
    group_by: SnapshotGroupCriterion,
    filter: impl FnMut(&SnapshotFile) -> bool + Send + Sync,
) -> RusticResult<Vec<(SnapshotGroup, Vec<SnapshotFile>)>> {
    let pb = &repo.pb;
    let dbe = repo.dbe();
    let p = pb.progress_counter("getting snapshots...");
    let groups = if ids.is_empty() {
        SnapshotFile::group_from_backend(dbe, filter, group_by, &p)?
    } else {
        let snaps = SnapshotFile::from_strs(dbe, ids, filter, &p)?;
        let mut result = Vec::new();
        for (group, snaps) in &snaps
            .into_iter()
            .chunk_by(|sn| SnapshotGroup::from_snapshot(sn, group_by))
        {
            result.push((group, snaps.collect()));
        }
        result
    };
    p.finish();

    Ok(groups)
}
