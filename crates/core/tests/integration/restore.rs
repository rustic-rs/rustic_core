use std::{fs, path::PathBuf, str::FromStr};

#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt;

use anyhow::Result;
use pretty_assertions::assert_eq;
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::{
    BackupOptions, LocalDestination, LsOptions, RestoreOptions, repofile::SnapshotFile,
};

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
