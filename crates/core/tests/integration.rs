use std::{
    env,
    fs::File,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::Result;
use flate2::read::GzDecoder;
use insta::assert_debug_snapshot;
use pretty_assertions::assert_eq;
use rustic_core::{
    repofile::SnapshotFile, BackupOptions, ConfigOptions, InMemoryBackend, KeyOptions,
    NoProgressBars, OpenStatus, PathList, Repository, RepositoryBackends, RepositoryOptions,
};
// uncomment for logging output
// use simplelog::{Config, SimpleLogger};
use tar::Archive;
use tempfile::{tempdir, TempDir};

fn set_up_repo() -> Result<Repository<NoProgressBars, OpenStatus>> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("test");
    let repo = Repository::new(&options, be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let repo = repo.init(&key_opts, config_opts)?;
    Ok(repo)
}

struct TestSource(TempDir);

impl TestSource {
    fn new(tmp: TempDir) -> Self {
        Self(tmp)
    }

    fn paths(&self) -> Result<PathList> {
        Ok(PathList::from_string(self.0.path().to_str().unwrap())?)
    }
}

fn set_up_testdata(path: impl AsRef<Path>) -> Result<TestSource> {
    let dir = tempdir()?;
    let path = Path::new("tests/testdata").join(path);
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.unpack(&dir)?;
    Ok(TestSource::new(dir))
}

// Parts of the snapshot summary we want to test against references
struct TestSummary<'a>(&'a SnapshotFile);

impl<'a> std::fmt::Debug for TestSummary<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // leave out info we expect to change:
        // Ids, times, tree sizes (as used uid/username is saved in trees)
        let mut b = f.debug_struct("TestSnap");
        b.field("hostname", &self.0.hostname);
        b.field("paths", &self.0.paths);
        b.field("label", &self.0.label);
        b.field("tags", &self.0.tags);

        let s = self.0.summary.as_ref().unwrap();
        b.field("files_new", &s.files_new);
        b.field("files_changed", &s.files_changed);
        b.field("files_unmodified", &s.files_unmodified);
        b.field("total_files_processed", &s.total_files_processed);
        b.field("total_bytes_processed", &s.total_bytes_processed);
        b.field("dirs_new", &s.dirs_new);
        b.field("dirs_changed", &s.dirs_changed);
        b.field("dirs_unmodified", &s.dirs_unmodified);
        b.field("total_dirs_processed", &s.total_dirs_processed);
        b.field("data_blobs", &s.data_blobs);
        b.field("tree_blobs", &s.tree_blobs);
        b.field("data_added_files", &s.data_added_files);
        b.field("data_added_files_packed", &s.data_added_files_packed);
        b.finish()
    }
}

#[test]
fn backup() -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;
    let source = set_up_testdata("backup-data.tar.gz")?;
    let paths = &source.paths()?;
    let repo = set_up_repo()?.to_indexed_ids()?;
    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap1));
    assert_eq!(snap1.parent, None);

    // get all snapshots and check them
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(vec![snap1.clone()], snaps);
    // save list of pack files
    let packs1: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let snap2 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap2));
    assert_eq!(snap2.parent, Some(snap1.id));
    assert_eq!(snap1.tree, snap2.tree);

    // get all snapshots and check them
    let mut snaps = repo.get_all_snapshots()?;
    snaps.sort_unstable();
    assert_eq!(vec![snap1, snap2], snaps);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs1, packs2);
    Ok(())
}

#[test]
fn backup_dry_run() -> Result<()> {
    let source = &set_up_testdata("backup-data.tar.gz")?;
    let paths = &source.paths()?;
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
    let packs: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs.len(), 0);
    let indexes: Vec<_> = repo.list(rustic_core::FileType::Index)?.collect();
    assert_eq!(indexes.len(), 0);

    // first real backup
    let opts = opts.dry_run(false);
    let snap1 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, snap1.tree);
    let packs: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_debug_snapshot!(TestSummary(&snap_dry_run));
    // check that no data has been added
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps, vec![snap1.clone()]);
    let packs_dry_run: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs_dry_run, packs);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second real backup
    let opts = opts.dry_run(false);
    let snap2 = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, snap2.tree);
    Ok(())
}
