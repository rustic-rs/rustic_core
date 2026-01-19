use std::{collections::BTreeMap, ffi::OsStr, path::PathBuf, str::FromStr};

use anyhow::Result;
use insta::Settings;
use jiff::Zoned;
use pretty_assertions::assert_eq;
use rstest::rstest;

use rustic_core::{
    BackupOptions, Excludes, LsOptions, NodeModification, RewriteOptions, RewriteTreesOptions,
    RusticResult, StringList,
    repofile::{Metadata, Node, SnapshotFile, SnapshotModification},
};

use super::{
    RepoOpen, TestSource, assert_with_win, insta_node_redaction, insta_snapshotfile_redaction,
    set_up_repo, tar_gz_testdata,
};

#[rstest]
fn test_rewrite(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
    insta_node_redaction: Settings,
) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let backup_opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let snapshot = repo.backup(&backup_opts, paths, SnapshotFile::default())?;

    let modification = SnapshotModification::default()
        .set_label("label".to_string())
        .set_time("2024-06-19[America/New_York]".parse::<Zoned>()?)
        .set_hostname("hostname".to_string())
        .add_tags(vec!["tag1,tag2".parse::<StringList>()?])
        .set_description("description".to_string())
        .set_delete_never(true);
    let mut rewrite_opts = RewriteOptions::default()
        .modification(modification)
        .forget(true);
    let rewrite_snaps = repo.rewrite_snapshots(vec![snapshot], &rewrite_opts)?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("rewrite-snapshots-first", &rewrite_snaps);
    });

    let snaps = repo.get_all_snapshots()?;
    assert_eq!(rewrite_snaps, snaps);

    let repo = repo.to_indexed()?;

    let rewrite_tree_params = RewriteTreesOptions::default()
        .excludes(Excludes::default().globs(vec!["!/test/0/0/9/6*".to_string()]))
        .node_modification(NodeModification::default());

    // with dry_run
    rewrite_opts.dry_run = true;
    let rewrite_snaps_dryrun =
        repo.rewrite_snapshots_and_trees(snaps.clone(), &rewrite_opts, &rewrite_tree_params)?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("rewrite-snapshots-second", &rewrite_snaps_dryrun);
    });
    let snaps_after_dryrun = repo.get_all_snapshots()?;
    assert_eq!(rewrite_snaps_dryrun, snaps_after_dryrun);

    rewrite_opts.dry_run = false;
    let rewrite_snaps =
        repo.rewrite_snapshots_and_trees(snaps, &rewrite_opts, &rewrite_tree_params)?;
    assert_eq!(rewrite_snaps_dryrun, rewrite_snaps);
    assert_eq!(rewrite_snaps.len(), 1);

    // re-read index
    let repo = repo.to_indexed_ids()?;

    // re-read index
    let repo = repo.to_indexed_ids()?;

    // test entries
    let mut node = Node::new_node(
        OsStr::new(""),
        rustic_core::repofile::NodeType::Dir,
        Metadata::default(),
    );
    node.subtree = Some(rewrite_snaps[0].tree);

    let entries: BTreeMap<_, _> = repo
        .ls(&node, &LsOptions::default())?
        .collect::<RusticResult<_>>()?;

    insta_node_redaction.bind(|| {
        assert_with_win("rewrite-nodes", &entries);
    });

    // backup with excludes
    let glob = "!".to_string() + source.path().to_str().unwrap() + "/0/0/9/6*"; // other exclude as we use as-path

    // #[cfg(windows)]
    let glob = glob.replace('\\', "/"); // correct windows paths for glob

    let excludes = Excludes::default().globs(vec![glob]);
    let backup_opts = backup_opts.excludes(excludes);
    let snapshot = repo.backup(&backup_opts, paths, SnapshotFile::default())?;
    // trees should be identical
    assert_eq!(snapshot.tree, rewrite_snaps[0].tree);

    Ok(())
}
