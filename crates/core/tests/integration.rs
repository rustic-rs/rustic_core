use anyhow::Result;
use flate2::read::GzDecoder;
use insta::assert_debug_snapshot;
use pretty_assertions::assert_eq;
use rstest::fixture;
use rstest::rstest;
use rustic_core::{
    repofile::SnapshotFile, BackupOptions, ConfigOptions, InMemoryBackend, KeyOptions,
    NoProgressBars, OpenStatus, PathList, Repository, RepositoryBackends, RepositoryOptions,
};
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
use tempfile::tempdir;

fn set_up_repo() -> Result<Repository<NoProgressBars, OpenStatus>> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("test");
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let repo = repo.init(&key_opts, config_opts)?;
    Ok(repo)
}

struct TestSource(PathBuf);

impl TestSource {
    fn new(tmp: impl Into<PathBuf>) -> Self {
        Self(tmp.into())
    }

    fn path_list(&self) -> PathList {
        PathList::from_iter(Some(self.0.clone()))
    }
}

#[fixture]
fn tar_gz_testdata() -> Result<TestSource> {
    let dir = tempdir()?;
    let path = Path::new("tests/fixtures/backup-data.tar.gz");
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.unpack(&dir)?;
    Ok(TestSource::new(dir.as_ref()))
}

#[fixture]
fn dir_testdata() -> Result<TestSource> {
    let path = Path::new("tests/fixtures/backup-data/");
    Ok(TestSource::new(path))
}

// Parts of the snapshot summary we want to test against references
struct TestSummary<'a>(&'a SnapshotFile);

impl<'a> std::fmt::Debug for TestSummary<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // leave out info we expect to change:
        // Ids, times, tree sizes (as used uid/username is saved in trees)
        let mut b = f.debug_struct("TestSnap");
        _ = b.field("hostname", &self.0.hostname);
        _ = b.field("paths", &self.0.paths);
        _ = b.field("label", &self.0.label);
        _ = b.field("tags", &self.0.tags);

        let s = self.0.summary.as_ref().unwrap();
        _ = b.field("files_new", &s.files_new);
        _ = b.field("files_changed", &s.files_changed);
        _ = b.field("files_unmodified", &s.files_unmodified);
        _ = b.field("total_files_processed", &s.total_files_processed);
        _ = b.field("total_bytes_processed", &s.total_bytes_processed);
        _ = b.field("dirs_new", &s.dirs_new);
        _ = b.field("dirs_changed", &s.dirs_changed);
        _ = b.field("dirs_unmodified", &s.dirs_unmodified);
        _ = b.field("total_dirs_processed", &s.total_dirs_processed);
        _ = b.field("data_blobs", &s.data_blobs);
        _ = b.field("tree_blobs", &s.tree_blobs);
        _ = b.field("data_added_files", &s.data_added_files);
        _ = b.field("data_added_files_packed", &s.data_added_files_packed);
        b.finish()
    }
}

#[rstest]
fn test_backup_with_dir_passes(dir_testdata: Result<TestSource>) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;
    let source = dir_testdata?;
    let paths = &source.path_list();

    let repo = set_up_repo()?.to_indexed_ids()?;
    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let first_backup = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&first_backup));
    assert_eq!(first_backup.parent, None);

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_backup.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let snap2 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap2));
    assert_eq!(snap2.parent, Some(first_backup.id));
    assert_eq!(first_backup.tree, snap2.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(vec![first_backup, snap2], all_snapshots);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs1, packs2);
    Ok(())
}

#[rstest]
fn test_backup_with_tar_gz_passes(tar_gz_testdata: Result<TestSource>) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;
    let source = tar_gz_testdata?;
    let paths = &source.path_list();

    let repo = set_up_repo()?.to_indexed_ids()?;
    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let first_backup = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&first_backup));
    assert_eq!(first_backup.parent, None);

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_backup.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let snap2 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap2));
    assert_eq!(snap2.parent, Some(first_backup.id));
    assert_eq!(first_backup.tree, snap2.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(vec![first_backup, snap2], all_snapshots);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs1, packs2);
    Ok(())
}

#[rstest]
fn test_backup_dry_run_with_tar_gz_passes(tar_gz_testdata: Result<TestSource>) -> Result<()> {
    let source = tar_gz_testdata?;
    let paths = &source.path_list();
    let repo = set_up_repo()?.to_indexed_ids()?;
    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default()
        .as_path(PathBuf::from_str("test")?)
        .dry_run(true);

    // dry-run backup
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap_dry_run));
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

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap_dry_run));
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
