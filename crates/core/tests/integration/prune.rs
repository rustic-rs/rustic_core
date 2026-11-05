use anyhow::Result;
use bytesize::ByteSize;
use jiff::Span;
use rstest::rstest;

use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, LimitOption, PathList, PruneOptions,
    repofile::{Chunker, SnapshotFile},
};

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};

#[rstest]
fn test_prune(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    #[values(
        ConfigOptions::default(),
        ConfigOptions::default()
        .set_chunker(Chunker::FixedSize)
        .set_chunk_size(ByteSize::b(100))
    )]
    opts: ConfigOptions,
    #[values(true, false)] instant_delete: bool,
    #[values(
        LimitOption::Percentage(0),
        LimitOption::Percentage(50),
        LimitOption::Unlimited
    )]
    max_unused: LimitOption,
    #[values(true, false)] fast_repack: bool,
) -> Result<()> {
    // Fixtures
    let (source, mut repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);
    _ = repo.apply_config(&opts)?;

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

    // remove first snapshot
    let repo = repo.drop_index();
    repo.delete_snapshots(&[snapshot1.id])?;

    // get prune plan
    let prune_opts = PruneOptions::default()
        .instant_delete(instant_delete)
        .max_unused(max_unused)
        .fast_repack(fast_repack);
    let plan = repo.prune_plan(&prune_opts)?;
    // TODO: Snapshot-test the plan (currently doesn't impl Serialize)
    // assert_ron_snapshot!("prune", plan);
    repo.prune(&prune_opts, plan)?;

    // run check
    let check_opts = CheckOptions::default().read_data(true);
    repo.check(check_opts)?.is_ok()?;

    if !instant_delete {
        // re-run if we only marked pack files. All marks should stay, as they are not old enough.
        let plan = repo.prune_plan(&prune_opts)?;
        repo.prune(&prune_opts, plan)?;
        repo.check(check_opts)?.is_ok()?;
        // re-run again with keep-delete = 0. Now all marked packs should be removed
        let prune_opts = prune_opts.keep_delete(Span::default());
        let plan = repo.prune_plan(&prune_opts)?;
        repo.prune(&prune_opts, plan)?;
        repo.check(check_opts)?.is_ok()?;
    }

    Ok(())
}

#[rstest]
fn test_prune_parallel_to_backup(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
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
    let check_opts = CheckOptions::default().read_data(true);
    let prune_opts = PruneOptions::default().max_unused(max_unused);

    // first backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9")));
    let snapshot1 = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/2")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // remove first snapshot
    repo.delete_snapshots(&[snapshot1.id])?;

    // get prune plan
    let plan = repo.prune_plan(&prune_opts)?;

    // run third backup in "parallel" to the prune
    let repo = repo.to_indexed_ids()?;
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/3")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;
    let repo = repo.drop_index();

    // now execute the plan
    repo.prune(&prune_opts, plan)?;
    // Check fails as the needed blob is marked for deletion here. TODO: Make check respect marked packs and print only a warning
    // repo.check(check_opts)?.is_ok()?;

    // run prune again -> should keep the marks
    let plan = repo.prune_plan(&prune_opts)?;
    repo.prune(&prune_opts, plan)?;
    // Check fails as the needed blob is still marked
    // repo.check(check_opts)?.is_ok()?;

    // re-run again with keep-delete = 0. Now packs should be recovered.
    let prune_opts = prune_opts.keep_delete(Span::default());
    let plan = repo.prune_plan(&prune_opts)?;
    repo.prune(&prune_opts, plan)?;
    repo.check(check_opts)?.is_ok()?;
    Ok(())
}

// TODO: This test fails as the second prune run tries to remove the same packs.
// #[rstest]
// fn test_prune_parallel(
//     tar_gz_testdata: Result<TestSource>,
//     set_up_repo: Result<RepoOpen>,
// ) -> Result<()> {
//     // Fixtures
//     let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

//     let opts = BackupOptions::default();
//     let check_opts = CheckOptions::default().read_data(true);
//     let prune_opts = PruneOptions::default();

//     // first backup
//     let paths = PathList::from_iter(Some(source.0.path().join("0/0/9")));
//     let snapshot1 = repo.backup(&opts, &paths, SnapshotFile::default())?;

//     // re-read index
//     let repo = repo.to_indexed_ids()?;
//     // second backup
//     let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/2")));
//     let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

//     // remove first snapshot
//     repo.delete_snapshots(&[snapshot1.id])?;

//     // get prune plan
//     let plan = repo.prune_plan(&prune_opts)?;
//     // get second prune plan
//     let plan2 = repo.prune_plan(&prune_opts)?;

//     // now execute the plan
//     repo.prune(&prune_opts, plan)?;
//     // execute second plan
//     repo.prune(&prune_opts, plan2)?;
//     repo.check(check_opts)?.is_ok()?;
//     Ok(())
// }
