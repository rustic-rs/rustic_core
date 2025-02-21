use std::time::Duration;

use anyhow::Result;
use rstest::rstest;

use rustic_core::{
    BackupOptions, CheckOptions, LimitOption, PathList, PruneOptions, repofile::SnapshotFile,
};

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};

#[rstest]
fn test_prune(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    #[values(true, false)] instant_delete: bool,
    #[values(
        LimitOption::Percentage(0),
        LimitOption::Percentage(50),
        LimitOption::Unlimited
    )]
    max_unused: LimitOption,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let opts = BackupOptions::default();

    // first backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9")));
    let snapshot1 = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/2")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // third backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/3")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // drop index
    let repo = repo.drop_index();
    repo.delete_snapshots(&[snapshot1.id])?;

    // get prune plan
    let prune_opts = PruneOptions::default()
        .instant_delete(instant_delete)
        .max_unused(max_unused)
        .keep_delete(Duration::ZERO);
    let plan = repo.prune_plan(&prune_opts)?;
    // TODO: Snapshot-test the plan (currently doesn't impl Serialize)
    // assert_ron_snapshot!("prune", plan);
    repo.prune(&prune_opts, plan)?;

    // run check
    let check_opts = CheckOptions::default().read_data(true);
    repo.check(check_opts)?;

    if !instant_delete {
        // re-run if we only marked pack files. As keep-delete = 0, they should be removed here
        let plan = repo.prune_plan(&prune_opts)?;
        repo.prune(&prune_opts, plan)?;
        repo.check(check_opts)?;
    }

    Ok(())
}
