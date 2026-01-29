use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use pretty_assertions::assert_eq;
use rstest::rstest;

use rustic_core::{BackupOptions, CheckOptions, CopySnapshot, repofile::SnapshotFile};

use super::{RepoOpen, TestSource, set_up_repo, tar_gz_testdata};

#[rstest]
fn test_copy(tar_gz_testdata: Result<TestSource>, set_up_repo: Result<RepoOpen>) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap = repo.backup(&opts, paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed()?;

    let target = super::set_up_repo()?;
    let relevant_snaps = target.relevant_copy_snapshots(|_| true, std::slice::from_ref(&snap))?;
    assert_eq!(
        relevant_snaps,
        vec![CopySnapshot {
            sn: snap.clone(),
            relevant: true
        }]
    );

    let target = target.to_indexed_ids()?;
    repo.copy(&target, Some(&snap))?;
    let check_opts = CheckOptions::default();
    target.check(check_opts)?.is_ok()?;

    Ok(())
}
