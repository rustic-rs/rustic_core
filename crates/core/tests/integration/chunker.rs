use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use bytesize::ByteSize;
use insta::{Settings, assert_ron_snapshot};
use rstest::rstest;

use rustic_core::{
    BackupOptions, ConfigOptions,
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
