use anyhow::Result;
use insta::assert_debug_snapshot;
use rstest::rstest;
use tempfile::tempdir;

use rustic_core::CheckOptions;

use crate::repo_from_fixture;

#[rstest]
#[case("repo-data-missing.tar.gz")]
#[case("repo-duplicates.tar.gz")]
// #[case("repo-index-missing-blob.tar.gz")] TODO: Activate when paths are identical for unix/windows
#[case("repo-index-missing.tar.gz")]
#[case("repo-mixed.tar.gz")]
#[case("repo-obsolete-index.tar.gz")]
#[case("repo-unreferenced-data.tar.gz")]
#[case("repo-unused-data-missing.tar.gz")]
fn test_check(#[case] repo_file: &str) -> Result<()> {
    // unpack repo
    let dir = tempdir()?;
    let repo = repo_from_fixture(&dir, repo_file)?;

    let opts = CheckOptions::default().read_data(true);
    let check_results = repo.check(opts)?;
    assert_debug_snapshot!(repo_file, check_results);

    Ok(())
}
