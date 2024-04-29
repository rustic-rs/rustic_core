//! Integration tests for the core library
//!
//! # How to update snapshots
//!
//! The CI pipeline is configured to run the tests with the `INSTA_UPDATE` environment variable set to `new`.
//! This means, it uploads the failed tests snapshots to the artifacts and you can download them and use them to update the snapshots.
//!
//! To update the snapshots, you download the artifacts and copy the files to the `tests/snapshots` directory.
//! Then you run `cargo insta review` to review the changes and accept them.
//!
//! # Redactions
//!
//! We use the `insta` crate to compare the actual output of the tests with the expected output.
//! Some data in the output changes every test run, we use insta's redactions to replace the actual values with placeholders.
//! We define the redactions inside `Settings` in the fixtures and bind them to the test functions. You can read more about
//! it [here](https://docs.rs/insta/latest/insta/struct.Settings.html).
//!
//! # Fixtures and Dependency Injection
//!
//! We use the `rstest` crate to define fixtures and dependency injection.
//! This allows us to define a set of fixtures that are used in multiple tests.
//! The fixtures are defined as functions with the `#[fixture]` attribute.
//! The tests that use the fixtures are defined as functions with the `#[rstest]` attribute.
//! The fixtures are passed as arguments to the test functions.

use anyhow::Result;
use flate2::read::GzDecoder;
use globset::Glob;
use insta::internals::{Content, ContentPath};
use insta::{assert_ron_snapshot, Settings};
use pretty_assertions::assert_eq;
use rstest::fixture;
use rstest::rstest;
use rustic_core::repofile::{Metadata, Node};
use rustic_core::{
    repofile::SnapshotFile, BackupOptions, ConfigOptions, KeyOptions, LsOptions, NoProgressBars,
    OpenStatus, PathList, Repository, RepositoryBackends, RepositoryOptions,
};
use rustic_core::{FindMatches, FindNode, RusticResult};
use serde::Serialize;

use rustic_testing::backend::in_memory_backend::InMemoryBackend;

use std::ffi::OsStr;
use std::{
    env,
    fs::File,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
// uncomment for logging output
// use simplelog::{Config, SimpleLogger};
use tar::Archive;
use tempfile::{tempdir, TempDir};

type RepoOpen = Repository<NoProgressBars, OpenStatus>;

#[derive(Debug)]
struct TestSource(TempDir);

impl TestSource {
    fn new(tmp: TempDir) -> Self {
        Self(tmp)
    }

    fn path_list(&self) -> PathList {
        PathList::from_iter(Some(self.0.path().to_path_buf()))
    }
}

#[fixture]
fn set_up_repo() -> Result<RepoOpen> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("test");
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let repo = repo.init(&key_opts, config_opts)?;
    Ok(repo)
}

// helper func to redact options, but still keep information about some/none
#[allow(clippy::needless_pass_by_value)] // we need exactly that function signature
fn handle_option(val: Content, _: ContentPath<'_>) -> String {
    if val.is_nil() {
        "[none]".to_string()
    } else {
        "[some]".to_string()
    }
}

#[fixture]
fn insta_summary_redaction() -> Settings {
    let mut settings = insta::Settings::clone_current();

    settings.add_redaction(".tree", "[tree_id]");
    settings.add_dynamic_redaction(".program_version", |val, _| {
        val.resolve_inner()
            .as_str()
            .map_or("[program_version]".to_string(), |v| {
                v.replace(env!("CARGO_PKG_VERSION"), "[rustic_core_version]")
            })
    });
    settings.add_redaction(".time", "[time]");
    settings.add_dynamic_redaction(".parent", handle_option);
    settings.add_redaction(".tags", "[tags]");
    settings.add_redaction(".id", "[id]");
    settings.add_redaction(".summary.backup_start", "[backup_start]");
    settings.add_redaction(".summary.backup_end", "[backup_end]");
    settings.add_redaction(".summary.backup_duration", "[backup_duration]");
    settings.add_redaction(".summary.total_duration", "[total_duration]");
    settings.add_redaction(".summary.data_added", "[data_added]");
    settings.add_redaction(".summary.data_added_packed", "[data_added_packed]");
    settings.add_redaction(
        ".summary.total_dirsize_processed",
        "[total_dirsize_processed]",
    );
    settings.add_redaction(
        ".summary.data_added_trees_packed",
        "[data_added_trees_packed]",
    );
    settings.add_redaction(".summary.data_added_trees", "[data_added_trees]");

    settings
}

#[fixture]
fn insta_tree_redaction() -> Settings {
    let mut settings = insta::Settings::clone_current();

    settings.add_redaction(".nodes[].inode", "[inode]");
    settings.add_redaction(".nodes[].device_id", "[device_id]");
    settings.add_redaction(".nodes[].uid", "[uid]");
    settings.add_redaction(".nodes[].user", "[user]");
    settings.add_redaction(".nodes[].gid", "[gid]");
    settings.add_redaction(".nodes[].group", "[group]");
    settings.add_dynamic_redaction(".nodes[].mode", handle_option);
    settings.add_dynamic_redaction(".nodes[].mtime", handle_option);
    settings.add_dynamic_redaction(".nodes[].atime", handle_option);
    settings.add_dynamic_redaction(".nodes[].ctime", handle_option);

    settings
}

#[fixture]
fn tar_gz_testdata() -> Result<TestSource> {
    let dir = tempdir()?;
    let path = Path::new("tests/fixtures/backup-data.tar.gz").canonicalize()?;
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.unpack(&dir)?;
    Ok(TestSource::new(dir))
}

// helper function to do windows-specific snapshots (needed e.g. if paths are contained in the snapshot)
fn assert_with_win<T: Serialize>(test: &str, snap: T) {
    #[cfg(windows)]
    assert_ron_snapshot!(format!("{test}-windows"), snap);
    #[cfg(not(windows))]
    assert_ron_snapshot!(format!("{test}-nix"), snap);
}
// Parts of the snapshot summary we want to test against references
//
// # Note
//
// We use a struct to avoid having to escape the field names in the snapshot
// we use insta redactions to replace the actual values with placeholders in case
// there are changes in the actual values
// Readme: https://insta.rs/docs/redactions/
#[derive(Serialize)]
struct TestSummary<'a>(&'a SnapshotFile);

#[rstest]
fn test_backup_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_summary_redaction: Settings,
    insta_tree_redaction: Settings,
) -> Result<()> {
    // uncomment for logging output
    // SimpleLogger::init(log::LevelFilter::Debug, Config::default())?;

    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // We can also bind to scope ( https://docs.rs/insta/latest/insta/struct.Settings.html#method.bind_to_scope )
    // But I think that can get messy with a lot of tests, also checking which settings are currently applied
    // will be probably harder
    insta_summary_redaction.bind(|| {
        assert_with_win("backup-tar-summary-first", TestSummary(&first_snapshot));
    });

    assert_eq!(first_snapshot.parent, None);

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree: rustic_core::repofile::Tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_tree_redaction.bind(|| {
        assert_with_win("backup-tar-tree", tree);
    });

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_snapshot.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_summary_redaction.bind(|| {
        assert_with_win("backup-tar-summary-second", TestSummary(&second_snapshot));
    });

    assert_eq!(second_snapshot.parent, Some(first_snapshot.id));
    assert_eq!(first_snapshot.tree, second_snapshot.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(vec![first_snapshot, second_snapshot], all_snapshots);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs1, packs2);

    // Check if snapshots can be retrieved
    let mut ids: Vec<_> = all_snapshots.iter().map(|sn| sn.id.to_string()).collect();
    let snaps = repo.get_snapshots(&ids)?;
    assert_eq!(snaps, all_snapshots);

    // reverse order
    all_snapshots.reverse();
    ids.reverse();
    let snaps = repo.get_snapshots(&ids)?;
    assert_eq!(snaps, all_snapshots);

    Ok(())
}

#[rstest]
fn test_backup_dry_run_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_summary_redaction: Settings,
    insta_tree_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default()
        .as_path(PathBuf::from_str("test")?)
        .dry_run(true);

    // dry-run backup
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_summary_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-first", TestSummary(&snap_dry_run));
    });

    // check that repo is still empty
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps.len(), 0);
    assert_eq!(repo.list(rustic_core::FileType::Pack)?.count(), 0);
    assert_eq!(repo.list(rustic_core::FileType::Index)?.count(), 0);

    // first real backup
    let opts = opts.dry_run(false);
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, first_snapshot.tree);
    let packs: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_tree_redaction.bind(|| {
        assert_with_win("dryrun-tar-tree", tree);
    });

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_summary_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-second", TestSummary(&snap_dry_run));
    });

    // check that no data has been added
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps, vec![first_snapshot]);
    let packs_dry_run: Vec<_> = repo.list(rustic_core::FileType::Pack)?.collect();
    assert_eq!(packs_dry_run, packs);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second real backup
    let opts = opts.dry_run(false);
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, second_snapshot.tree);
    Ok(())
}

#[rstest]
fn test_ls(tar_gz_testdata: Result<TestSource>, set_up_repo: Result<RepoOpen>) -> Result<()> {
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

    let _entries: Vec<_> = repo
        .ls(&node, &LsOptions::default())?
        .collect::<RusticResult<_>>()?;
    // TODO: Snapshot-test entries
    // assert_ron_snapshot!("ls", entries);
    Ok(())
}

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
