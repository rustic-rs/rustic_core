use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use bytes::Bytes;
use insta::Settings;
use pretty_assertions::assert_eq;
use rstest::rstest;

use rustic_core::{BackupOptions, repofile::SnapshotFile, vfs::Vfs};

use super::{
    RepoOpen, TestSource, assert_with_win, insta_node_redaction, set_up_repo, tar_gz_testdata,
};

#[rstest]
fn test_vfs(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_node_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);
    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    // backup test-data
    let snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed()?;
    // create Vfs
    let node = repo.node_from_snapshot_and_path(&snapshot, "")?;
    let vfs = Vfs::from_dir_node(&node);

    // test reading a directory using vfs
    let path: PathBuf = ["test", "0", "tests"].iter().collect();
    let entries = vfs.dir_entries_from_path(&repo, &path)?;
    insta_node_redaction.bind(|| {
        assert_with_win("vfs", &entries);
    });

    // test reading a file from the repository
    let path: PathBuf = ["test", "0", "tests", "testfile"].iter().collect();
    let node = vfs.node_from_path(&repo, &path)?;
    let file = repo.open_file(&node)?;

    let data = repo.read_file_at(&file, 0, 21)?; // read full content
    assert_eq!(Bytes::from("This is a test file.\n"), &data);
    assert_eq!(data, repo.read_file_at(&file, 0, 4096)?); // read beyond file end
    assert_eq!(Bytes::new(), repo.read_file_at(&file, 25, 1)?); // offset beyond file end
    assert_eq!(Bytes::from("test"), repo.read_file_at(&file, 10, 4)?); // read partial content

    // test reading an empty file from the repository
    let path: PathBuf = ["test", "0", "tests", "empty-file"].iter().collect();
    let node = vfs.node_from_path(&repo, &path)?;
    let file = repo.open_file(&node)?;
    assert_eq!(Bytes::new(), repo.read_file_at(&file, 0, 0)?); // empty files
    Ok(())
}
