// don't warn about try_from paths conversion on unix
#![allow(clippy::unnecessary_fallible_conversions)]

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Result;
use globset::Glob;
use insta::assert_debug_snapshot;
use rstest::rstest;

use rustic_core::{
    BackupOptions, FindMatches, FindNode,
    repofile::{Node, SnapshotFile},
};
use typed_path::UnixPath;

use super::{RepoOpen, TestSource, assert_with_win, set_up_repo, tar_gz_testdata};

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
    let not_found =
        repo.find_nodes_from_path(vec![snapshot.tree], UnixPath::new("not_existing"))?;
    assert_with_win("find-nodes-not-found", not_found);
    // test non-existing match
    let glob = Glob::new("not_existing")?.compile_matcher();
    let not_found = repo.find_matching_nodes(vec![snapshot.tree], &|path, _| {
        glob.is_match(PathBuf::try_from(path).unwrap())
    })?;
    assert_debug_snapshot!("find-matching-nodes-not-found", not_found);

    // test existing path
    let FindNode { matches, .. } =
        repo.find_nodes_from_path(vec![snapshot.tree], UnixPath::new("test/0/tests/testfile"))?;
    assert_with_win("find-nodes-existing", matches);
    // test existing match
    let glob = Glob::new("testfile")?.compile_matcher();
    let match_func = |path: &UnixPath, _: &Node| {
        glob.is_match(PathBuf::try_from(path).unwrap())
            || path
                .file_name()
                .is_some_and(|f| glob.is_match(Path::new(&String::from_utf8(f.to_vec()).unwrap())))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    assert_debug_snapshot!("find-matching-existing", (paths, matches));
    // test existing match
    let glob = Glob::new("testfile*")?.compile_matcher();
    let match_func = |path: &UnixPath, _: &Node| {
        glob.is_match(PathBuf::try_from(path).unwrap())
            || path
                .file_name()
                .is_some_and(|f| glob.is_match(Path::new(&String::from_utf8(f.to_vec()).unwrap())))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    assert_debug_snapshot!("find-matching-wildcard-existing", (paths, matches));
    Ok(())
}
