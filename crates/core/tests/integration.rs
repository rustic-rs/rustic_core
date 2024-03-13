use anyhow::Result;
use flate2::read::GzDecoder;
use insta::assert_ron_snapshot;
use pretty_assertions::assert_eq;
use rstest::fixture;
use rstest::rstest;
use rustic_core::{
    repofile::SnapshotFile, BackupOptions, ConfigOptions, InMemoryBackend, KeyOptions,
    NoProgressBars, OpenStatus, PathList, Repository, RepositoryBackends, RepositoryOptions,
};
use serde_derive::Serialize;

use std::{
    env,
    fs::File,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
// uncomment for logging output
// use simplelog::{Config, SimpleLogger};
use tar::Archive;
use tempfile::{tempdir, TempDir};

type RepoOpen = Repository<NoProgressBars, OpenStatus>;

#[fixture]
fn set_up_repo() -> Result<RepoOpen> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("test");
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let repo = repo.init(&key_opts, config_opts)?;
    Ok(repo)
}

#[derive(Debug)]
struct TestSource(TempDir);

impl TestSource {
    fn new(tmp: TempDir) -> Self {
        Self(tmp)
    }

    fn path_list(&self) -> PathList {
        PathList::from_iter(Some(self.0.path().to_path_buf()))
    }
}

#[fixture]
fn tar_gz_testdata() -> Result<TestSource> {
    let dir = tempdir()?;
    let path = Path::new("tests/fixtures/backup-data.tar.gz").canonicalize()?;
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.unpack(&dir)?;
    Ok(TestSource::new(dir))
}

// Parts of the snapshot summary we want to test against references
//
// # Note
//
// We use a struct to avoid having to escape the field names in the snapshot
// we use insta redactions to replace the actual values with placeholders in case
// there are changes in the actual values
// Readme: https://insta.rs/docs/redactions/
#[derive(Serialize)]
struct TestSummary<'a>(&'a SnapshotFile);

#[rstest]
fn test_backup_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
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

    #[cfg(windows)]
    assert_ron_snapshot!(
        "backup-tar-summary-first-windows",
        TestSummary(&first_snapshot),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );

    #[cfg(not(windows))]
    assert_ron_snapshot!("backup-tar-summary-first-nix",
    TestSummary(&first_snapshot),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    });

    assert_eq!(first_snapshot.parent, None);

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree: rustic_core::repofile::Tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    #[cfg(windows)]
    assert_ron_snapshot!(
        "backup-tar-tree-windows",
        tree,
        {
            ".nodes[].ctime" => "[ctime]",
            ".nodes[].content" => "[content_id]",
        }
    );

    #[cfg(not(windows))]
    assert_ron_snapshot!(
        "backup-tar-tree-nix",
        tree,
        {
            ".nodes[].ctime" => "[ctime]",
            ".nodes[].content" => "[content_id]",
        }
    );

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_snapshot.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    #[cfg(windows)]
    assert_ron_snapshot!(
        "backup-tar-summary-second-windows",
        TestSummary(&second_snapshot),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );
    #[cfg(not(windows))]
    assert_ron_snapshot!(
        "backup-tar-summary-second-nix",
        TestSummary(&second_snapshot),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );

    assert_eq!(second_snapshot.parent, Some(first_snapshot.id));
    assert_eq!(first_snapshot.tree, second_snapshot.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(vec![first_snapshot, second_snapshot], all_snapshots);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs1, packs2);
    Ok(())
}

#[rstest]
fn test_backup_dry_run_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
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

    #[cfg(windows)]
    assert_ron_snapshot!(
        "dryrun-tar-summary-first-windows",
        TestSummary(&snap_dry_run),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );

    #[cfg(not(windows))]
    assert_ron_snapshot!("dryrun-tar-summary-first-nix",
    TestSummary(&snap_dry_run),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    });

    // check that repo is still empty
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps.len(), 0);
    assert_eq!(repo.list(rustic_core::FileType::Pack)?.count(), 0);
    assert_eq!(repo.list(rustic_core::FileType::Index)?.count(), 0);

    // first real backup
    let opts = opts.dry_run(false);
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, first_snapshot.tree);
    let packs: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    #[cfg(windows)]
    assert_ron_snapshot!(
        "dryrun-tar-tree-windows",
        tree,
        {
            ".nodes[].ctime" => "[ctime]",
            ".nodes[].content" => "[content_id]",
        }
    );

    #[cfg(not(windows))]
    assert_ron_snapshot!(
        "dryrun-tar-tree-nix",
        tree,
        {
            ".nodes[].ctime" => "[ctime]",
            ".nodes[].content" => "[content_id]",
        }
    );

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    #[cfg(windows)]
    assert_ron_snapshot!(
        "dryrun-tar-summary-second-windows",
        TestSummary(&snap_dry_run),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );

    #[cfg(not(windows))]
    assert_ron_snapshot!("dryrun-tar-summary-second-nix",
    TestSummary(&snap_dry_run),
    {
        ".tree" => "[tree_id]",
        ".program_version" => "[version]",
        ".time" => "[time]",
        ".tags" => "[tags]",
        ".id" => "[id]",
        ".summary.backup_start" => "[backup_start]",
        ".summary.backup_end" => "[backup_end]",
        ".summary.backup_duration" => "[backup_duration]",
        ".summary.total_duration" => "[total_duration]",
    }
    );

    // check that no data has been added
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps, vec![first_snapshot]);
    let packs_dry_run: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs_dry_run, packs);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second real backup
    let opts = opts.dry_run(false);
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, second_snapshot.tree);
    Ok(())
}
