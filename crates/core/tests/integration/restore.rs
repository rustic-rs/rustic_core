use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

use bytesize::ByteSize;
#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt;

use anyhow::Result;
use pretty_assertions::assert_eq;
use rstest::rstest;
use rustic_backend::LocalBackend;
use tempfile::tempdir;

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};
use rustic_core::{
    BackupOptions, ConfigOptions, Credentials, FileType, KeyOptions, LocalDestination, LsOptions,
    Repository, RepositoryBackends, RepositoryOptions, RestoreOptions, WriteBackend,
    repofile::{Chunker, MasterKey, SnapshotFile},
};

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

#[test]
fn restore_continues_after_pack_read_failure() -> Result<()> {
    let source = tempdir()?;
    fs::write(source.path().join("unavailable"), vec![1; 1024])?;
    fs::write(source.path().join("survives"), vec![2; 1024])?;

    let repo_dir = tempdir()?;
    let backend = Arc::new(LocalBackend::new(
        repo_dir
            .path()
            .to_str()
            .expect("repository path is valid utf-8"),
        None,
    )?);
    let backends = RepositoryBackends::new(backend.clone(), None);
    let config_opts = ConfigOptions::default()
        .set_chunker(Chunker::FixedSize)
        .set_chunk_size(ByteSize::b(1024))
        .set_datapack_size(ByteSize::b(1))
        .set_datapack_growfactor(0)
        .set_datapack_size_limit(ByteSize::b(1));
    let repo = Repository::new(&RepositoryOptions::default(), &backends)?
        .init(
            &Credentials::Masterkey(MasterKey::new()),
            &KeyOptions::default(),
            &config_opts,
        )?
        .to_indexed_ids()?;
    let snapshot = repo.backup(
        &BackupOptions::default().as_path(PathBuf::from("/")),
        &rustic_core::PathList::from_iter(Some(source.path().to_path_buf())),
        SnapshotFile::default(),
    )?;
    let repo = repo.to_indexed()?;

    let unavailable =
        repo.node_from_snapshot_path(&format!("{}:unavailable", snapshot.id), |_| true)?;
    let unavailable_id = unavailable
        .content
        .as_ref()
        .and_then(|content| content.first())
        .expect("unavailable file has content");
    let unavailable_pack = repo.get_index_entry(unavailable_id)?.pack;

    let survives = repo.node_from_snapshot_path(&format!("{}:survives", snapshot.id), |_| true)?;
    let survives_id = survives
        .content
        .as_ref()
        .and_then(|content| content.first())
        .expect("surviving file has content");
    assert_ne!(unavailable_pack, repo.get_index_entry(survives_id)?.pack);

    backend.remove(FileType::Pack, &unavailable_pack, false)?;

    let node = repo.node_from_snapshot_path(&snapshot.id.to_string(), |_| true)?;
    let ls = repo.ls(&node, &LsOptions::default())?;
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

    let err = repo
        .restore(plan, &restore_opts, ls, &dest)
        .expect_err("restore must report the unavailable pack");
    assert!(
        err.to_string()
            .contains("Remaining readable data was restored")
    );
    assert_eq!(
        fs::read(restore_dir.path().join("survives"))?,
        vec![2; 1024]
    );

    Ok(())
}
