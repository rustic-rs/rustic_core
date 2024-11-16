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
use bytes::Bytes;
use flate2::read::GzDecoder;
use globset::Glob;
use insta::{
    assert_ron_snapshot,
    internals::{Content, ContentPath},
    Settings,
};
use pretty_assertions::assert_eq;
use rstest::{fixture, rstest};
use rustic_core::{
    repofile::{Metadata, Node, PackId, SnapshotFile},
    BackupOptions, CheckOptions, CommandInput, ConfigOptions, FindMatches, FindNode, FullIndex,
    IndexedFull, IndexedStatus, KeyOptions, LimitOption, LsOptions, NoProgressBars, OpenStatus,
    ParentOptions, PathList, PruneOptions, Repository, RepositoryBackends, RepositoryOptions,
    RusticResult, SnapshotGroupCriterion, SnapshotOptions, StringList,
};
use serde::Serialize;

use rustic_testing::backend::in_memory_backend::InMemoryBackend;

use std::{collections::BTreeMap, ffi::OsStr};
use std::{
    env,
    fs::File,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
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
fn insta_snapshotfile_redaction() -> Settings {
    let mut settings = Settings::clone_current();

    settings.add_redaction(".**.tree", "[tree_id]");
    settings.add_dynamic_redaction(".**.program_version", |val, _| {
        val.resolve_inner()
            .as_str()
            .map_or("[program_version]".to_string(), |v| {
                v.replace(env!("CARGO_PKG_VERSION"), "[rustic_core_version]")
            })
    });
    settings.add_redaction(".**.time", "[time]");
    settings.add_dynamic_redaction(".**.parent", handle_option);
    settings.add_redaction(".**.id", "[id]");
    settings.add_redaction(".**.original", "[original]");
    settings.add_redaction(".**.hostname", "[hostname]");
    settings.add_redaction(".**.command", "[command]");
    settings.add_redaction(".**.summary.backup_start", "[backup_start]");
    settings.add_redaction(".**.summary.backup_end", "[backup_end]");
    settings.add_redaction(".**.summary.backup_duration", "[backup_duration]");
    settings.add_redaction(".**.summary.total_duration", "[total_duration]");
    settings.add_redaction(".**.summary.data_added", "[data_added]");
    settings.add_redaction(".**.summary.data_added_packed", "[data_added_packed]");
    settings.add_redaction(
        ".**.summary.total_dirsize_processed",
        "[total_dirsize_processed]",
    );
    settings.add_redaction(
        ".**.summary.data_added_trees_packed",
        "[data_added_trees_packed]",
    );
    settings.add_redaction(".**.summary.data_added_trees", "[data_added_trees]");

    settings
}

#[fixture]
fn insta_node_redaction() -> Settings {
    let mut settings = Settings::clone_current();

    settings.add_redaction(".**.inode", "[inode]");
    settings.add_redaction(".**.device_id", "[device_id]");
    settings.add_redaction(".**.uid", "[uid]");
    settings.add_redaction(".**.user", "[user]");
    settings.add_redaction(".**.gid", "[gid]");
    settings.add_redaction(".**.group", "[group]");
    settings.add_dynamic_redaction(".**.mode", handle_option);
    settings.add_dynamic_redaction(".**.mtime", handle_option);
    settings.add_dynamic_redaction(".**.atime", handle_option);
    settings.add_dynamic_redaction(".**.ctime", handle_option);
    settings.add_dynamic_redaction(".**.subtree", handle_option);

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

#[test]
fn repo_with_commands() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let command: CommandInput = "echo test".parse()?;
    let warm_up: CommandInput = "echo %id".parse()?;
    let options = RepositoryOptions::default()
        .password_command(command)
        .warm_up_command(warm_up);
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let _repo = repo.init(&key_opts, config_opts)?;
    Ok(())
}

#[rstest]
fn test_backup_with_tar_gz_passes(
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
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // first backup
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // We can also bind to scope ( https://docs.rs/insta/latest/insta/struct.Settings.html#method.bind_to_scope )
    // But I think that can get messy with a lot of tests, also checking which settings are currently applied
    // will be probably harder
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-first", &first_snapshot);
    });

    assert_eq!(first_snapshot.parent, None);

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree: rustic_core::repofile::Tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_node_redaction.bind(|| {
        assert_with_win("backup-tar-tree", tree);
    });

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![first_snapshot.clone()], all_snapshots);
    // save list of pack files
    let packs1: Vec<PackId> = repo.list()?.collect();

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let second_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-second", &second_snapshot);
    });

    assert_eq!(second_snapshot.parent, Some(first_snapshot.id));
    assert_eq!(first_snapshot.tree, second_snapshot.tree);

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs2);

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // third backup with tags and explicitly given parent
    let snap = SnapshotOptions::default()
        .tags([StringList::from_str("a,b")?])
        .to_snapshot()?;
    let opts = opts.parent_opts(ParentOptions::default().parent(second_snapshot.id.to_string()));
    let third_snapshot = repo.backup(&opts, paths, snap)?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-summary-third", &third_snapshot);
    });
    assert_eq!(third_snapshot.parent, Some(second_snapshot.id));
    assert_eq!(third_snapshot.tree, second_snapshot.tree);

    // get all snapshots and check them
    let mut all_snapshots = repo.get_all_snapshots()?;
    all_snapshots.sort_unstable();
    assert_eq!(
        vec![first_snapshot, second_snapshot, third_snapshot],
        all_snapshots
    );

    // pack files should be unchanged
    let packs2: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs2);
    let packs3: Vec<_> = repo.list()?.collect();
    assert_eq!(packs1, packs3);

    // Check if snapshots can be retrieved
    let mut ids: Vec<_> = all_snapshots.iter().map(|sn| sn.id.to_string()).collect();
    let snaps = repo.get_snapshots(&ids)?;
    assert_eq!(snaps, all_snapshots);

    // reverse order
    all_snapshots.reverse();
    ids.reverse();
    let snaps = repo.update_snapshots(snaps, &ids)?;
    assert_eq!(snaps, all_snapshots);

    // get snapshot group
    let group_by = SnapshotGroupCriterion::new().tags(true);
    let mut groups = repo.get_snapshot_group(&[], group_by, |_| true)?;

    // sort groups to get unique result
    groups.iter_mut().for_each(|(_, snaps)| snaps.sort());
    groups.sort_by_key(|(group, _)| group.tags.clone());

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-groups", &groups);
    });

    // filter snapshots by tag
    let filter = |snap: &SnapshotFile| snap.tags.contains("a");
    let snaps = repo.get_matching_snapshots(filter)?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("backup-tar-matching-snaps", &snaps);
    });

    Ok(())
}

#[rstest]
fn test_backup_dry_run_with_tar_gz_passes(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
    insta_node_redaction: Settings,
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

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-first", &snap_dry_run);
    });

    // check that repo is still empty
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps.len(), 0);
    assert_eq!(repo.list::<PackId>()?.count(), 0);
    assert_eq!(repo.list::<PackId>()?.count(), 0);

    // first real backup
    let opts = opts.dry_run(false);
    let first_snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;
    assert_eq!(snap_dry_run.tree, first_snapshot.tree);
    let packs: Vec<_> = repo.list::<PackId>()?.collect();

    // tree of first backup
    // re-read index
    let repo = repo.to_indexed_ids()?;
    let tree = repo.node_from_path(first_snapshot.tree, Path::new("test/0/tests"))?;
    let tree = repo.get_tree(&tree.subtree.expect("Sub tree"))?;

    insta_node_redaction.bind(|| {
        assert_with_win("dryrun-tar-tree", tree);
    });

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second dry-run backup
    let opts = opts.dry_run(true);
    let snap_dry_run = repo.backup(&opts, paths, SnapshotFile::default())?;

    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("dryrun-tar-summary-second", &snap_dry_run);
    });

    // check that no data has been added
    let snaps = repo.get_all_snapshots()?;
    assert_eq!(snaps, vec![first_snapshot]);
    let packs_dry_run: Vec<PackId> = repo.list()?.collect();
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
fn test_backup_stdin_command(
    set_up_repo: Result<RepoOpen>,
    insta_snapshotfile_redaction: Settings,
) -> Result<()> {
    // Fixtures
    let repo = set_up_repo?.to_indexed_ids()?;
    let paths = PathList::from_string("-")?;

    let cmd: CommandInput = "echo test".parse()?;
    let opts = BackupOptions::default()
        .stdin_filename("test")
        .stdin_command(cmd);
    // backup data from cmd
    let snapshot = repo.backup(&opts, &paths, SnapshotFile::default())?;
    insta_snapshotfile_redaction.bind(|| {
        assert_with_win("stdin-command-summary", &snapshot);
    });

    // re-read index
    let repo = repo.to_indexed()?;

    // check content
    let node = repo.node_from_snapshot_path("latest:test", |_| true)?;
    let mut content = Vec::new();
    repo.dump(&node, &mut content)?;
    assert_eq!(content, b"test\n");
    Ok(())
}

#[rstest]
fn test_ls_and_read(
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

    // test reading a file from the repository
    let repo = repo.to_indexed()?;
    let path: PathBuf = ["test", "0", "tests", "testfile"].iter().collect();
    let node = entries.get(&path).unwrap();
    let file = repo.open_file(node)?;

    let data = repo.read_file_at(&file, 0, 21)?; // read full content
    assert_eq!(Bytes::from("This is a test file.\n"), &data);
    let data2 = repo.read_file_at(&file, 0, 4096)?; // read beyond file end
    assert_eq!(data2, &data);
    assert_eq!(Bytes::new(), repo.read_file_at(&file, 25, 1)?); // offset beyond file end
    assert_eq!(Bytes::from("test"), repo.read_file_at(&file, 10, 4)?); // read partial content

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

#[rstest]
fn test_prune(
    tar_gz_testdata: Result<TestSource>,
    set_up_repo: Result<RepoOpen>,
    #[values(true, false)] instant_delete: bool,
    #[values(
        LimitOption::Percentage(0),
        LimitOption::Percentage(50),
        LimitOption::Unlimited
    )]
    max_unused: LimitOption,
) -> Result<()> {
    // Fixtures
    let (source, repo) = (tar_gz_testdata?, set_up_repo?.to_indexed_ids()?);

    let opts = BackupOptions::default();

    // first backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9")));
    let snapshot1 = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // second backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/2")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // re-read index
    let repo = repo.to_indexed_ids()?;
    // third backup
    let paths = PathList::from_iter(Some(source.0.path().join("0/0/9/3")));
    let _ = repo.backup(&opts, &paths, SnapshotFile::default())?;

    // drop index
    let repo = repo.drop_index();
    repo.delete_snapshots(&[snapshot1.id])?;

    // get prune plan
    let prune_opts = PruneOptions::default()
        .instant_delete(instant_delete)
        .max_unused(max_unused)
        .keep_delete(Duration::ZERO);
    let plan = repo.prune_plan(&prune_opts)?;
    // TODO: Snapshot-test the plan (currently doesn't impl Serialize)
    // assert_ron_snapshot!("prune", plan);
    repo.prune(&prune_opts, plan)?;

    // run check
    let check_opts = CheckOptions::default().read_data(true);
    repo.check(check_opts)?;

    if !instant_delete {
        // re-run if we only marked pack files. As keep-delete = 0, they should be removed here
        let plan = repo.prune_plan(&prune_opts)?;
        repo.prune(&prune_opts, plan)?;
        repo.check(check_opts)?;
    }

    Ok(())
}

/// Verifies that users can create wrappers around repositories
/// without resorting to generics. The rationale is that such
/// types can be used to dynamically open, store, and cache repos.
///
/// See issue #277 for more context.
#[test]
fn test_wrapping_in_new_type() -> Result<()> {
    struct Wrapper(Repository<NoProgressBars, IndexedStatus<FullIndex, OpenStatus>>);

    impl Wrapper {
        fn new() -> Result<Self> {
            Ok(Self(set_up_repo()?.to_indexed()?))
        }
    }

    /// Fake function that "does something" with a fully indexed repo
    /// (without actually relying on any functionality for the test)
    fn use_repo(_: &impl IndexedFull) {}

    let collection: Vec<Wrapper> = vec![Wrapper::new()?, Wrapper::new()?];

    collection.iter().map(|r| &r.0).for_each(use_repo);

    Ok(())
}
