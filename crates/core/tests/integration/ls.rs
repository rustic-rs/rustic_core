use std::{collections::BTreeMap, ffi::OsStr};
use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use insta::Settings;
use rstest::rstest;

use rustic_core::{
    repofile::{Metadata, Node, SnapshotFile},
    BackupOptions, LsOptions, RusticResult,
};

use super::{
    assert_with_win, insta_node_redaction, set_up_repo, tar_gz_testdata, RepoOpen, TestSource,
};

#[rstest]
fn test_ls(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_node_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);
    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);
    // backup test-data
    let snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // test non-existing entries
    let mut node = Node::new_node(
        OsStr::new(""),
        rustic_core::repofile::NodeType::Dir,
        Metadata::default(),
    );
    node.subtree = Some(snapshot.tree);

    // re-read index
    let repo = repo.to_indexed_ids()?;

    let entries: BTreeMap<_, _> = repo
        .ls(&node, &LsOptions::default())?
        .collect::<RusticResult<_>>()?;

    insta_node_redaction.bind(|| {
        assert_with_win("ls", &entries);
    });

    Ok(())
}
