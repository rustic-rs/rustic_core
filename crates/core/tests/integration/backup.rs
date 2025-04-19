use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Result;
use insta::Settings;
use pretty_assertions::assert_eq;
use rstest::rstest;

use rustic_core::{
    BackupOptions, CommandInput, ParentOptions, PathList, SnapshotGroupCriterion, SnapshotOptions,
    StringList,
    repofile::{PackId, SnapshotFile},
};

use super::{
    RepoOpen, TestSource, assert_with_win, insta_node_redaction, insta_snapshotfile_redaction,
    set_up_repo, tar_gz_testdata,
};

#[rstest]
fn test_backup_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
    insta_node_redaction: Settings,
) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // We can also bind to scope ( https://docs.rs/insta/latest/insta/struct.Settings.html#method.bind_to_scope )
    // But I think that can get messy with a lot of tests, also checking which settings are currently applied
    // will be probably harder
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-first", &first_snapshot);
    });

    assert_eq!(first_snapshot.parent, None);

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree: rustic_core::repofile::Tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_node_redaction.bind(|| {
        assert_with_win("backup-tar-tree", tree);
    });

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_snapshot.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<PackId> = repo.list()?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-second", &second_snapshot);
    });

    assert_eq!(second_snapshot.parent, Some(first_snapshot.id));
    assert_eq!(first_snapshot.tree, second_snapshot.tree);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs2);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // third backup with tags and explicitly given parent
    let snap = SnapshotOptions::default()
        .tags([StringList::from_str("a,b")?])
        .to_snapshot()?;
    let opts = opts.parent_opts(ParentOptions::default().parent(second_snapshot.id.to_string()));
    let third_snapshot = repo.backup(&opts, paths, snap)?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-third", &third_snapshot);
    });
    assert_eq!(third_snapshot.parent, Some(second_snapshot.id));
    assert_eq!(third_snapshot.tree, second_snapshot.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(
        vec![first_snapshot, second_snapshot, third_snapshot],
        all_snapshots
    );

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs2);
    let packs3: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs3);

    // Check if snapshots can be retrieved
    let mut ids: Vec<_> = all_snapshots.iter().map(|sn| sn.id.to_string()).collect();
    let snaps = repo.get_snapshots(&ids)?;
    assert_eq!(snaps, all_snapshots);

    // reverse order
    all_snapshots.reverse();
    ids.reverse();
    let snaps = repo.update_snapshots(snaps, &ids)?;
    assert_eq!(snaps, all_snapshots);

    // get snapshot group
    let group_by = SnapshotGroupCriterion::new().tags(true);
    let mut groups = repo.get_snapshot_group(&[], group_by, |_| true)?;

    // sort groups to get unique result
    groups.iter_mut().for_each(|(_, snaps)| snaps.sort());
    groups.sort_by_key(|(group, _)| group.tags.clone());

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-groups", &groups);
    });

    // filter snapshots by tag
    let filter = |snap: &SnapshotFile| snap.tags.contains("a");
    let snaps = repo.get_matching_snapshots(filter)?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-matching-snaps", &snaps);
    });

    Ok(())
}

#[rstest]
fn test_backup_dry_run_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
    insta_node_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default()
        .as_path(PathBuf::from_str("test")?)
        .dry_run(true);

    // dry-run backup
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-first", &snap_dry_run);
    });

    // check that repo is still empty
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps.len(), 0);
    assert_eq!(repo.list::<PackId>()?.count(), 0);
    assert_eq!(repo.list::<PackId>()?.count(), 0);

    // first real backup
    let opts = opts.dry_run(false);
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, first_snapshot.tree);
    let packs: Vec<_> = repo.list::<PackId>()?.collect();

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_node_redaction.bind(|| {
        assert_with_win("dryrun-tar-tree", tree);
    });

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-second", &snap_dry_run);
    });

    // check that no data has been added
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps, vec![first_snapshot]);
    let packs_dry_run: Vec<PackId> = repo.list()?.collect();
    assert_eq!(packs_dry_run, packs);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second real backup
    let opts = opts.dry_run(false);
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, second_snapshot.tree);
    Ok(())
}

#[rstest]
fn test_backup_stdin_command(
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let repo = set_up_repo?.to_indexed_ids()?;
    let paths = PathList::from_string("-")?;

    let cmd: CommandInput = "echo test".parse()?;
    let opts = BackupOptions::default()
        .stdin_filename("test")
        .stdin_command(cmd);
    // backup data from cmd
    let snapshot = repo.backup(&opts, &paths, SnapshotFile::default())?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("stdin-command-summary", &snapshot);
    });

    // re-read index
    let repo = repo.to_indexed()?;

    // check content
    let node = repo.node_from_snapshot_path("latest:test", |_| true)?;
    let mut content = Vec::new();
    repo.dump(&node, &mut content)?;
    assert_eq!(content, b"test\n");
    Ok(())
}
