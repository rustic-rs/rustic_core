#![allow(missing_docs)]

use std::path::PathBuf;
use std::str::FromStr;

use crate::tar_gz_testdata;

use super::set_up_repo;
use anyhow::Result;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use rstest::{fixture, rstest};
use rustic_core::repofile::SnapshotFile;
use rustic_core::{
    BackupOptions, IdIndex, IndexedStatus, NoProgressBars, OpenStatus, Repository,
    SnapshotGroupCriterion,
};

#[fixture]
#[once]
fn repo_and_snapshots() -> (
    Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
    Vec<SnapshotFile>,
) {
    let repo = set_up_repo().unwrap().to_indexed_ids().unwrap();
    let source = tar_gz_testdata().unwrap();

    let snapshot_timestamp = [
        Timestamp::from_second(1_752_483_600)
            .unwrap()
            .to_zoned(TimeZone::UTC),
        Timestamp::from_second(1_752_483_700)
            .unwrap()
            .to_zoned(TimeZone::UTC),
        Timestamp::from_second(1_752_483_800)
            .unwrap()
            .to_zoned(TimeZone::UTC),
    ];
    let mut snapshot_files = Vec::new();

    // we use as_path to not depend on the actual tempdir
    let backup_options = BackupOptions::default().as_path(PathBuf::from_str("test").unwrap());
    for snap_ts in snapshot_timestamp {
        let snapshot_file = repo
            .backup(
                &backup_options,
                &source.path_list(),
                SnapshotFile {
                    time: snap_ts,
                    ..Default::default()
                },
            )
            .unwrap();
        snapshot_files.push(snapshot_file);
    }

    (repo, snapshot_files)
}

#[rstest]
fn test_get_snapshot_group_no_ids(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, snapshots) = repo_and_snapshots;

    let res = repo.get_snapshot_group(&[], SnapshotGroupCriterion::default(), |_| true)?;

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), snapshots.len());

    Ok(())
}

#[rstest]
fn test_get_snapshot_group_wrong_id(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) {
    let (repo, _snapshots) = repo_and_snapshots;

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
}

#[rstest]
fn test_get_snapshot_group_latest_id(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, snapshots) = repo_and_snapshots;
    let res = repo.get_snapshot_group(
        &[String::from("latest")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 1);

    // latest => most recent
    assert_eq!(res[0].1[0], snapshots[2]);
    Ok(())
}

#[rstest]
fn test_get_snapshot_group_latest_n_id(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, snapshots) = repo_and_snapshots;

    let res = repo.get_snapshot_group(
        &[String::from("latest~2")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 1);
    // latest~2 is oldest
    assert_eq!(res[0].1[0], snapshots[0]);

    let res = repo.get_snapshot_group(
        &[String::from("latest~2"), String::from("latest~0")],
        SnapshotGroupCriterion::default(),
        |_| true,
    )?;
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1.len(), 2);
    // latest~2 is oldest
    assert_eq!(res[0].1[0], snapshots[0]);
    // latest~0 is latest
    assert_eq!(res[0].1[1], snapshots[2]);
    Ok(())
}

#[rstest]
fn test_get_snapshot_from_str_short_id(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, _snapshots) = repo_and_snapshots;

    let snap_original = repo.get_all_snapshots()?[0].clone();

    let id_str = &snap_original.id.to_string();
    let short_id = &id_str[..8];

    let snap_short_id = repo.get_snapshot_from_str(short_id, |_| true)?;

    assert_eq!(snap_short_id, snap_original);
    Ok(())
}

#[rstest]
fn test_get_snapshot_from_str_latest(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, snapshots) = repo_and_snapshots;

    let snap_latest = repo.get_snapshot_from_str("latest", |_| true)?;
    assert_eq!(snap_latest, snapshots[2]);
    let snap_latest_1 = repo.get_snapshot_from_str("latest~1", |_| true)?;
    assert_eq!(snap_latest_1, snapshots[1]);
    Ok(())
}

#[rstest]
fn test_get_snapshots_from_strs_latest(
    repo_and_snapshots: &(
        Repository<NoProgressBars, IndexedStatus<IdIndex, OpenStatus>>,
        Vec<SnapshotFile>,
    ),
) -> Result<()> {
    let (repo, snapshots) = repo_and_snapshots;

    let snap_latest = repo.get_snapshots_from_strs(&["latest", "latest~1"], |_| true)?;
    assert_eq!(snap_latest[0], snapshots[2]);
    assert_eq!(snap_latest[1], snapshots[1]);
    Ok(())
}
