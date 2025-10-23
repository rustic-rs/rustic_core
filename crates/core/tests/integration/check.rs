use std::{fs::File, path::Path, sync::Arc};

use anyhow::Result;
use flate2::read::GzDecoder;
use insta::assert_debug_snapshot;
use rstest::rstest;
use tar::Archive;
use tempfile::tempdir;

use rustic_backend::LocalBackend;
use rustic_core::{CheckOptions, Repository, RepositoryBackends, RepositoryOptions};

#[rstest]
#[case("repo-data-missing.tar.gz")]
#[case("repo-duplicates.tar.gz")]
#[case("repo-index-missing-blob.tar.gz")]
#[case("repo-index-missing.tar.gz")]
#[case("repo-mixed.tar.gz")]
#[case("repo-obsolete-index.tar.gz")]
#[case("repo-unreferenced-data.tar.gz")]
#[case("repo-unused-data-missing.tar.gz")]
fn test_check(#[case] repo_file: String) -> Result<()> {
    // unpack repo
    let dir = tempdir()?;
    let path = Path::new("tests/fixtures/").join(&repo_file);
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.unpack(&dir)?;

    let be = LocalBackend::new(dir.path().join("repo").to_str().unwrap(), None)?;
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("geheim");
    let repo = Repository::new(&options, &be)?.open()?;

    let opts = CheckOptions::default().read_data(true);
    let check_results = repo.check(opts)?;
    assert_debug_snapshot!(repo_file, check_results);

    Ok(())
}
