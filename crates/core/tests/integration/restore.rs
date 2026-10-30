use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt;

use anyhow::Result;
use pretty_assertions::assert_eq;
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::{
    BackupOptions, ConfigOptions, Credentials, FileType, IndexedFull, KeyOptions,
    LocalDestination, LsOptions, PathList, ReadBackend, Repository, RepositoryBackends,
    RepositoryOptions, RestoreOptions, WriteBackend,
    repofile::{MasterKey, SnapshotFile},
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};

#[rstest]
#[cfg(not(windows))]
fn test_restore_preserves_hardlinks(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    let _snapshot = repo.backup(&opts, &source.path_list(), SnapshotFile::default())?;

    let repo = repo.to_indexed()?;
    let node = repo.node_from_snapshot_path("latest", |_| true)?;
    let ls_opts = LsOptions::default();
    let ls = repo.ls(&node, &ls_opts)?;

    let restore_dir = tempdir()?;
    let dest = LocalDestination::new(
        restore_dir
            .path()
            .to_str()
            .expect("restore path is valid utf-8"),
        true,
        !node.is_dir(),
    )?;
    let restore_opts = RestoreOptions::default();
    let plan = repo.prepare_restore(&restore_opts, ls.clone(), &dest, false)?;
    repo.restore(plan, &restore_opts, ls, &dest)?;

    let hardlink = restore_dir.path().join("test/0/tests/testfile-hardlink");
    let linked = restore_dir.path().join("test/0/tests/testfile");
    let symlink = restore_dir.path().join("test/0/tests/testfile-symlink");

    let hardlink_meta = fs::metadata(&hardlink)?;
    let linked_meta = fs::metadata(&linked)?;
    assert_eq!(hardlink_meta.dev(), linked_meta.dev());
    assert_eq!(hardlink_meta.ino(), linked_meta.ino());
    assert_eq!(hardlink_meta.nlink(), 2);
    assert_eq!(linked_meta.nlink(), 2);
    assert_eq!(fs::read_to_string(&hardlink)?, fs::read_to_string(&linked)?);
    assert_eq!(fs::read_link(&symlink)?, PathBuf::from("testfile"));

    Ok(())
}

/// Back up a directory containing `a.txt` with content "hello" as `test`
fn backup_hello(repo: RepoOpen) -> Result<Repository<impl IndexedFull>> {
    let src = tempdir()?;
    fs::write(src.path().join("a.txt"), "hello")?;
    let repo = repo.to_indexed_ids()?;
    let opts = BackupOptions::default().as_path(PathBuf::from("test"));
    let paths = PathList::from_iter(Some(src.path().to_path_buf()));
    _ = repo.backup(&opts, &paths, SnapshotFile::default())?;
    Ok(repo.to_indexed()?)
}

/// Restore `snap_path` into `dir`
fn restore_into<S: IndexedFull>(
    repo: &Repository<S>,
    snap_path: &str,
    dir: &std::path::Path,
) -> Result<()> {
    let node = repo.node_from_snapshot_path(snap_path, |_| true)?;
    let ls = repo.ls(&node, &LsOptions::default())?;
    let dest = LocalDestination::new(dir.to_str().unwrap(), true, !node.is_dir())?;
    let opts = RestoreOptions::default();
    let plan = repo.prepare_restore(&opts, ls.clone(), &dest, false)?;
    repo.restore(plan, &opts, ls, &dest)?;
    Ok(())
}

#[test]
fn test_restore_damaged_pack_returns_error() -> Result<()> {
    let be = Arc::new(InMemoryBackend::new());
    let backends = RepositoryBackends::new(be.clone(), None);
    let repo = Repository::new(&RepositoryOptions::default().no_cache(true), &backends)?.init(
        &Credentials::Masterkey(MasterKey::new()),
        &KeyOptions::default(),
        &ConfigOptions::default(),
    )?;
    let repo = backup_hello(repo)?;

    // damage the data blob of a.txt, keeping the length of its pack
    let node = repo.node_from_snapshot_path("latest:test/a.txt", |_| true)?;
    let entry = repo.get_index_entry(&node.content.unwrap()[0])?;
    let mut pack = be.read_full(FileType::Pack, &entry.pack)?.to_vec();
    pack[usize::try_from(entry.location.offset + entry.location.length / 2)?] ^= 0xff;
    be.remove(FileType::Pack, &entry.pack, false)?;
    be.write_bytes(FileType::Pack, &entry.pack, false, pack.into())?;

    let restore_dir = tempdir()?;
    assert!(restore_into(&repo, "latest:test", restore_dir.path()).is_err());
    Ok(())
}

#[cfg(unix)]
#[rstest]
fn test_restore_over_read_only_file_returns_error(set_up_repo: Result<RepoOpen>) -> Result<()> {
    use std::fs::{OpenOptions, Permissions};
    use std::os::unix::fs::PermissionsExt;

    let repo = backup_hello(set_up_repo?)?;

    let restore_dir = tempdir()?;
    let file = restore_dir.path().join("a.txt");
    fs::write(&file, "old")?;
    fs::set_permissions(&file, Permissions::from_mode(0o444))?;
    fs::set_permissions(restore_dir.path(), Permissions::from_mode(0o555))?;

    // e.g. running as root; mode bits are not enforced
    let writable = OpenOptions::new().write(true).open(&file).is_ok();
    let res = (!writable).then(|| restore_into(&repo, "latest:test", restore_dir.path()));

    fs::set_permissions(restore_dir.path(), Permissions::from_mode(0o755))?;
    if let Some(res) = res {
        assert!(res.is_err());
    }
    Ok(())
}
