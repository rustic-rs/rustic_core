use std::{fs, io::Write, path::PathBuf, str::FromStr};

use anyhow::Result;
use bytesize::ByteSize;
use pretty_assertions::assert_eq;
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::{
    BackupOptions, ConfigOptions, IndexedFullStatus, PathList, Repository,
    repofile::{Chunker, SnapshotFile},
};

use super::{RepoOpen, set_up_repo};

/// Build a deterministic byte payload of the requested length.
fn payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).expect("251 always fits in u8"))
        .collect()
}

/// Backup a single file with the given content into `repo`, configuring the
/// fixed-size chunker so the file reliably splits into multiple blobs.
///
/// Returns the repository in the [`IndexedFullStatus`] state along with the
/// snapshot path that points at the backed-up file.
fn backup_single_file(
    repo: RepoOpen,
    name: &str,
    data: &[u8],
) -> Result<(Repository<IndexedFullStatus>, String)> {
    let dir = tempdir()?;
    let file_path = dir.path().join(name);
    fs::File::create(&file_path)?.write_all(data)?;

    let mut repo = repo.to_indexed_ids()?;
    let config = ConfigOptions::default()
        .set_chunker(Chunker::FixedSize)
        .set_chunk_size(ByteSize(4096));
    assert!(repo.apply_config(&config)?);

    let paths = PathList::from_iter([file_path]);
    let opts = BackupOptions::default().as_path(PathBuf::from_str(name)?);
    let _snapshot = repo.backup(&opts, &paths, SnapshotFile::default())?;

    Ok((repo.to_indexed()?, format!("latest:{name}")))
}

#[rstest]
fn test_dump_multi_blob_matches_source(set_up_repo: Result<RepoOpen>) -> Result<()> {
    let data = payload(64 * 1024);
    let (repo, snapshot_path) = backup_single_file(set_up_repo?, "file.bin", &data)?;
    let node = repo.node_from_snapshot_path(&snapshot_path, |_| true)?;

    // Sanity: the configured chunker must have produced more than one blob,
    // otherwise the parallel path is never taken.
    let blob_count = node.content.as_ref().map_or(0, Vec::len);
    assert!(
        blob_count > 1,
        "expected the test file to span multiple blobs, got {blob_count}",
    );

    let mut out = Vec::new();
    repo.dump(&node, &mut out)?;
    assert_eq!(out, data);
    Ok(())
}

#[rstest]
fn test_dump_default_options_match_source(set_up_repo: Result<RepoOpen>) -> Result<()> {
    let data = payload(32 * 1024);
    let (repo, snapshot_path) = backup_single_file(set_up_repo?, "file.bin", &data)?;
    let node = repo.node_from_snapshot_path(&snapshot_path, |_| true)?;

    let mut out = Vec::new();
    repo.dump(&node, &mut out)?;
    assert_eq!(out, data);
    Ok(())
}
