use std::{collections::HashSet, fs, path::PathBuf, str::FromStr};

use anyhow::Result;
use bytesize::ByteSize;
use insta::{Settings, assert_ron_snapshot};
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, PathList,
    repofile::{Chunker, SnapshotFile},
};

use super::{RepoOpen, TestSource, insta_snapshotfile_redaction, set_up_repo, tar_gz_testdata};

#[rstest]
fn test_chunker_params(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, mut repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // set fixed size chunker with a given chunk size
    let config_opts = ConfigOptions::default()
        .set_chunker(Chunker::FixedSize)
        .set_chunk_size(ByteSize(8000));

    assert!(repo.apply_config(&config_opts)?);

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    let snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // We can also bind to scope ( https://docs.rs/insta/latest/insta/struct.Settings.html#method.bind_to_scope )
    // But I think that can get messy with a lot of tests, also checking which settings are currently applied
    // will be probably harder
    insta_snapshotfile_redaction.bind(|| {
        assert_ron_snapshot!("chunker-fixedsize", &snapshot);
    });

    Ok(())
}

#[rstest]
#[case(Chunker::Rabin)]
#[case(Chunker::FixedSize)]
fn test_backup_zeros(set_up_repo: Result<RepoOpen>, #[case] chunker: Chunker) -> Result<()> {
    let mut repo = set_up_repo?.to_indexed_ids()?;
    let config_opts = ConfigOptions::default().set_chunker(chunker);
    _ = repo.apply_config(&config_opts)?;

    // Runs of zeros between data, so that zero chunks follow and precede other chunks.
    let mut data = vec![0; 8 << 20];
    let mut x: u32 = 1;
    for range in [0..100_000, 3_000_123..3_100_000, (8 << 20) - 5..8 << 20] {
        for b in &mut data[range] {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *b = (x >> 24) as u8 | 1;
        }
    }
    let dir = tempdir()?;
    let file = dir.path().join("zeros");
    fs::write(&file, &data)?;

    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    _ = repo.backup(
        &opts,
        &PathList::from_iter(Some(dir.path().to_path_buf())),
        SnapshotFile::default(),
    )?;

    // The zero chunks are stored once, under the hash of their content.
    let repo = repo.to_indexed()?;
    repo.check(CheckOptions::default().read_data(true))?
        .is_ok()?;
    let node = repo.node_from_snapshot_path("latest:test/zeros", |_| true)?;
    let content = node.content.as_ref().unwrap();
    assert!(content.iter().collect::<HashSet<_>>().len() < content.len());
    let mut restored = Vec::new();
    repo.dump(&node, &mut restored)?;
    assert_eq!(restored, data);
    Ok(())
}
