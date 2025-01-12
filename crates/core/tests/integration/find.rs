use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Result;
use globset::Glob;
use rstest::rstest;

use rustic_core::{
    repofile::{Node, SnapshotFile},
    BackupOptions, FindMatches, FindNode,
};

use super::{assert_with_win, set_up_repo, tar_gz_testdata, RepoOpen, TestSource};

#[rstest]
fn test_find(tar_gz_testdata: Result<TestSource>, set_up_repo: Result<RepoOpen>) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);
    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    // backup test-data
    let snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;

    // test non-existing path
    let not_found = repo.find_nodes_from_path(vec![snapshot.tree], Path::new("not_existing"))?;
    assert_with_win("find-nodes-not-found", not_found);
    // test non-existing match
    let glob = Glob::new("not_existing")?.compile_matcher();
    let not_found =
        repo.find_matching_nodes(vec![snapshot.tree], &|path, _| glob.is_match(path))?;
    assert_with_win("find-matching-nodes-not-found", not_found);

    // test existing path
    let FindNode { matches, .. } =
        repo.find_nodes_from_path(vec![snapshot.tree], Path::new("test/0/tests/testfile"))?;
    assert_with_win("find-nodes-existing", matches);
    // test existing match
    let glob = Glob::new("testfile")?.compile_matcher();
    let match_func = |path: &Path, _: &Node| {
        glob.is_match(path) || path.file_name().is_some_and(|f| glob.is_match(f))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    assert_with_win("find-matching-existing", (paths, matches));
    // test existing match
    let glob = Glob::new("testfile*")?.compile_matcher();
    let match_func = |path: &Path, _: &Node| {
        glob.is_match(path) || path.file_name().is_some_and(|f| glob.is_match(f))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    assert_with_win("find-matching-wildcard-existing", (paths, matches));
    Ok(())
}
