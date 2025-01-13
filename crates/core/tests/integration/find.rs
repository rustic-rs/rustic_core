// don't warn about try_from paths conversion on unix
#![allow(clippy::unnecessary_fallible_conversions)]

use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use globset::Glob;
use rstest::rstest;

use rustic_core::{
    BackupOptions, FindMatches, FindNode,
    repofile::{Node, SnapshotFile},
    util::{GlobMatcherExt, SerializablePath},
};
use typed_path::UnixPath;

use super::{RepoOpen, TestSource, assert_ron, set_up_repo, tar_gz_testdata};

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
    assert_ron("find-nodes-not-found", not_found);
    // test non-existing match
    let glob = Glob::new("not_existing")?.compile_matcher();
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &|path, _| glob.is_unix_match(path))?;
    assert!(paths.is_empty());
    assert_eq!(matches, [[]]);

    // test existing path
    let FindNode { matches, .. } =
        repo.find_nodes_from_path(vec![snapshot.tree], UnixPath::new("test/0/tests/testfile"))?;
    assert_ron("find-nodes-existing", matches);
    // test existing match
    let glob = Glob::new("testfile")?.compile_matcher();
    let match_func = |path: &UnixPath, _: &Node| {
        glob.is_unix_match(path) || path.file_name().is_some_and(|f| glob.is_unix_match(f))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    let paths: Vec<_> = paths.into_iter().map(SerializablePath).collect();
    assert_ron("find-matching-existing", (paths, matches));
    // test existing match
    let glob = Glob::new("testfile*")?.compile_matcher();
    let match_func = |path: &UnixPath, _: &Node| {
        glob.is_unix_match(path) || path.file_name().is_some_and(|f| glob.is_unix_match(f))
    };
    let FindMatches { paths, matches, .. } =
        repo.find_matching_nodes(vec![snapshot.tree], &match_func)?;
    let paths: Vec<_> = paths.into_iter().map(SerializablePath).collect();
    assert_ron("find-matching-wildcard-existing", (paths, matches));
    Ok(())
}
