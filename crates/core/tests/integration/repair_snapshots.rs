use std::collections::BTreeMap;

use anyhow::Result;
use insta::{Settings, assert_ron_snapshot};
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::{CheckOptions, LsOptions, RepairSnapshotsOptions, RusticResult};

use crate::{insta_node_redaction, repo_from_fixture};

#[rstest]
// #[case("repo-data-missing.tar.gz")]
#[case("repo-index-missing-blob.tar.gz")]
fn test_repair_snapshots(#[case] repo_file: &str, insta_node_redaction: Settings) -> Result<()> {
    // unpack repo
    let dir = tempdir()?;
    let repo = repo_from_fixture(&dir, repo_file)?;

    let check_opts = CheckOptions::default().read_data(true);
    assert!(repo.check(check_opts)?.is_ok().is_err());

    let snapshots = repo.get_all_snapshots()?;
    let repo = repo.to_indexed()?;

    let opts = RepairSnapshotsOptions::default()
        .delete(true)
        .suffix(".repaired");
    repo.repair_snapshots(&opts, snapshots, false)?;

    // reread index
    let repo = repo.to_indexed()?;

    // check should now return no critical error anymore
    assert!(repo.check(check_opts)?.is_ok().is_ok());

    let node = repo.node_from_snapshot_path("latest:home/thinkpad/data", |_sn| true)?;
    let entries: BTreeMap<_, _> = repo
        .ls(&node, &LsOptions::default())?
        .collect::<RusticResult<_>>()?;

    insta_node_redaction.bind(|| {
        assert_ron_snapshot!("repair-snapshots", &entries);
    });

    Ok(())
}
