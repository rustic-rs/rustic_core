#![allow(missing_docs)]

use std::path::PathBuf;
use std::str::FromStr;

use crate::{RepoOpen, TestSource, tar_gz_testdata};

use super::set_up_repo;
use anyhow::Result;
use chrono::DateTime;
use rstest::rstest;
use rustic_core::repofile::SnapshotFile;
use rustic_core::{BackupOptions, SnapshotGroupCriterion};

#[rstest]
fn test_get_snapshot_group_no_ids(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    // second backup
    let snap2_ts = DateTime::from_timestamp(1_752_483_700, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap2_ts,
            ..Default::default()
        },
    )?;
    let res = repo.get_snapshot_group(&[], SnapshotGroupCriterion::default(), |_| true)?;

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 2);

    Ok(())
}

#[rstest]
fn test_get_snapshot_group_wrong_id(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    let res = repo.get_snapshot_group(
        &[String::from("wrong_id_that_is_out_of_format")],
        SnapshotGroupCriterion::default(),
        |_| true,
    );
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.to_string()
            .contains("No suitable id found for `wrong_id_that_is_out_of_format`.")
    );
    Ok(())
}

#[rstest]
fn test_get_snapshot_group_latest_id(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    // second backup
    let snap2_ts = DateTime::from_timestamp(1_752_483_700, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap2_ts,
            ..Default::default()
        },
    )?;
    let res = repo.get_snapshot_group(
        &[String::from("latest")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 1);
    // latest => most recent
    assert_eq!(res[0].1[0].time, snap2_ts);
    Ok(())
}

#[rstest]
fn test_get_snapshot_group_latest_n_id(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    // second backup
    let snap2_ts = DateTime::from_timestamp(1_752_483_700, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap2_ts,
            ..Default::default()
        },
    )?;

    // third backup
    let snap3_ts = DateTime::from_timestamp(1_752_483_800, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap3_ts,
            ..Default::default()
        },
    )?;

    let res = repo.get_snapshot_group(
        &[String::from("latest~2")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 1);
    // latest~2 is "third" oldest
    assert_eq!(res[0].1[0].time, snap1_ts);

    let res = repo.get_snapshot_group(
        &[String::from("latest~2"), String::from("latest~0")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 2);
    // latest~2 is "third" oldest
    assert_eq!(res[0].1[0].time, snap1_ts);
    // latest~0 is latest
    assert_eq!(res[0].1[1].time, snap3_ts);
    Ok(())
}

#[rstest]
fn test_get_snapshot_from_str_short_id(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let _ = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    let snap_original = repo.get_all_snapshots()?[0].clone();

    let id_str = &snap_original.id.to_string();
    let short_id = &id_str[..8];

    let snap_short_id = repo.get_snapshot_from_str(short_id, |_| true)?;

    assert_eq!(snap_short_id, snap_original);
    Ok(())
}

#[rstest]
fn test_get_snapshot_from_str_latest(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let snap1 = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    // second backup
    let snap2_ts = DateTime::from_timestamp(1_752_483_700, 0).unwrap().into();
    let snap2 = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap2_ts,
            ..Default::default()
        },
    )?;

    let snap_latest = repo.get_snapshot_from_str("latest", |_| true)?;
    assert_eq!(snap_latest, snap2);
    let snap_latest_1 = repo.get_snapshot_from_str("latest~1", |_| true)?;
    assert_eq!(snap_latest_1, snap1);
    Ok(())
}

#[rstest]
fn test_get_snapshots_from_strs_latest(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
) -> Result<()> {
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snap1_ts = DateTime::from_timestamp(1_752_483_600, 0).unwrap().into();
    let snap1 = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap1_ts,
            ..Default::default()
        },
    )?;

    // second backup
    let snap2_ts = DateTime::from_timestamp(1_752_483_700, 0).unwrap().into();
    let snap2 = repo.backup(
        &opts,
        paths,
        SnapshotFile {
            time: snap2_ts,
            ..Default::default()
        },
    )?;

    let snap_latest = repo.get_snapshots_from_strs(&["latest", "latest~1"], |_| true)?;
    assert_eq!(snap_latest[0], snap2);
    assert_eq!(snap_latest[1], snap1);
    Ok(())
}
