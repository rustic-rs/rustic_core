use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use rstest::rstest;

use rustic_core::{BackupOptions, ConfigOptions, PruneOptions, repofile::SnapshotFile};

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};

#[rstest]
fn test_append_only(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, mut repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    // set repo to append-only mode
    let config_opts = ConfigOptions::default().set_append_only(true);
    assert!(repo.apply_config(&config_opts)?);

    let paths = &source.path_list();

    // backup should still work
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    let snap = repo.backup(&opts, paths, SnapshotFile::default())?;

    // deleting snapshots should fail
    assert!(repo.delete_snapshots(&[snap.id]).is_err());

    // pruning should fail
    let prune_options = PruneOptions::default();
    let prune_plan = repo.prune_plan(&prune_options)?;
    assert!(repo.prune(&prune_options, prune_plan).is_err());

    // modifying config should fail
    let config_opts = ConfigOptions::default().set_extra_verify(false);
    assert!(repo.apply_config(&config_opts).is_err());

    // disable append-only-mode
    let config_opts = ConfigOptions::default().set_append_only(false);
    assert!(repo.apply_config(&config_opts)?);

    // operations should now work
    repo.delete_snapshots(&[snap.id])?;
    let prune_plan = repo.prune_plan(&prune_options)?;
    repo.prune(&prune_options, prune_plan)?;
    let config_opts = ConfigOptions::default().set_extra_verify(false);
    _ = repo.apply_config(&config_opts)?;

    Ok(())
}
