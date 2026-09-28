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
mod integration {
    mod append_only;
    mod backup;
    mod check;
    mod chunker;
    mod copy;
    mod dump;
    mod find;
    mod hotcold;
    mod key;
    mod ls;
    mod prune;
    mod repair_snapshots;
    mod restore;
    mod rewrite;
    mod snapshots;
    mod vfs;
    use super::*;
}

use std::{env, fs, fs::File, path::Path, sync::Arc};

use anyhow::Result;
use flate2::read::GzDecoder;
use insta::{
    Settings, assert_ron_snapshot,
    internals::{Content, ContentPath},
};
use rstest::fixture;
use rustic_backend::LocalBackend;
use serde::Serialize;
use tar::Archive;
use tempfile::{TempDir, tempdir};
// uncomment for logging output
// use simplelog::{Config, SimpleLogger};

use rustic_core::{
    BackupOptions, CommandInput, ConfigOptions, CredentialOptions, Credentials, IndexedFull,
    IndexedFullStatus, KeyOptions, OpenStatus, PathList, Repository, RepositoryBackends,
    RepositoryOptions,
    repofile::{MasterKey, SnapshotFile},
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

type RepoOpen = Repository<OpenStatus>;

#[derive(Debug)]
struct TestSource(TempDir);

impl TestSource {
    fn new(tmp: TempDir) -> Self {
        Self(tmp)
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn path_list(&self) -> PathList {
        PathList::from_iter(Some(self.0.path().to_path_buf()))
    }
}

#[fixture]
fn set_up_repo() -> Result<RepoOpen> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default();
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let repo = repo.init(
        &Credentials::Masterkey(MasterKey::new()),
        &key_opts,
        config_opts,
    )?;
    Ok(repo)
}

fn repo_from_fixture(dir: &TempDir, repo_file: &str) -> Result<RepoOpen> {
    let path = Path::new("tests/fixtures/").join(repo_file);
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.unpack(dir)?;

    let be = LocalBackend::new(dir.path().join("repo").to_str().unwrap(), None)?;
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default();
    let repo = Repository::new(&options, &be)?.open(&Credentials::password("geheim"))?;
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
        val.resolve_inner().as_str().map_or_else(
            || "[program_version]".to_string(),
            |v| v.replace(env!("CARGO_PKG_VERSION"), "[rustic_core_version]"),
        )
    });
    settings.add_redaction(".**.time", "[time]");
    settings.add_dynamic_redaction(".**.parent", handle_option);
    settings.add_redaction(".**.parents", "[parents]");
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
    let options = RepositoryOptions::default().warm_up_command(warm_up);
    let credentials = CredentialOptions::default()
        .password_command(command)
        .credentials()?
        .unwrap();
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let _repo = repo.init(&credentials, &key_opts, config_opts)?;
    Ok(())
}

/// Verifies that users can create wrappers around repositories
/// without resorting to generics. The rationale is that such
/// types can be used to dynamically open, store, and cache repos.
///
/// See issue #277 for more context.
#[test]
fn test_wrapping_in_new_type() -> Result<()> {
    struct Wrapper(Repository<IndexedFullStatus>);

    impl Wrapper {
        fn new() -> Result<Self> {
            Ok(Self(set_up_repo()?.to_indexed()?))
        }
    }

    /// Fake function that "does something" with a fully indexed repo
    /// (without actually relying on any functionality for the test)
    fn use_repo(_: &Repository<impl IndexedFull>) {}

    let collection: Vec<Wrapper> = vec![Wrapper::new()?, Wrapper::new()?];

    collection.iter().map(|r| &r.0).for_each(use_repo);

    Ok(())
}

const STALE_PACK_IDS_LOAD: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STALE_PACK_FULL_LOAD: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Names of cached pack files. The sweep only considers 64-character hex names.
fn cached_pack_names(repo_cache: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let data = repo_cache.join("data");
    if !data.is_dir() {
        return Ok(names);
    }
    for prefix in fs::read_dir(data)? {
        let prefix = prefix?.path();
        if !prefix.is_dir() {
            continue;
        }
        for file in fs::read_dir(prefix)? {
            let file = file?;
            if !file.file_type()?.is_file() {
                continue;
            }
            let name = file.file_name();
            let name = name.to_string_lossy();
            if name.len() == 64 {
                names.push(name.into_owned());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn plant_cached_pack(repo_cache: &Path, id_hex: &str) -> Result<()> {
    let path = repo_cache.join("data").join(&id_hex[..2]).join(id_hex);
    fs::create_dir_all(path.parent().expect("pack path has a parent"))?;
    fs::write(path, b"not-a-pack")?;
    Ok(())
}

/// Loading the index drops cached packs that are no longer in it.
///
/// `to_indexed_ids` is the backup path. `to_indexed` is the full index path.
#[test]
fn index_load_drops_cached_packs_missing_from_the_index() -> Result<()> {
    let cache_dir = tempdir()?;
    let source = tempdir()?;
    fs::write(source.path().join("file.txt"), b"hello")?;

    let backends = RepositoryBackends::new(Arc::new(InMemoryBackend::new()), None);
    let options = RepositoryOptions::default().cache_dir(cache_dir.path().to_path_buf());
    let repo = Repository::new(&options, &backends)?.init(
        &Credentials::Masterkey(MasterKey::new()),
        &KeyOptions::default(),
        &ConfigOptions::default(),
    )?;
    let repo_cache = cache_dir.path().join(repo.config().id.to_hex().as_str());

    let repo = repo.to_indexed_ids()?;
    let _snapshot = repo.backup(
        &BackupOptions::default(),
        &PathList::from_iter(Some(source.path().to_path_buf())),
        SnapshotFile::default(),
    )?;

    let real_packs = cached_pack_names(&repo_cache)?;
    assert!(
        !real_packs.is_empty(),
        "backup should cache the tree pack it writes"
    );

    plant_cached_pack(&repo_cache, STALE_PACK_IDS_LOAD)?;
    let with_stale = cached_pack_names(&repo_cache)?;
    assert!(with_stale.iter().any(|name| name == STALE_PACK_IDS_LOAD));

    let repo = repo.to_indexed_ids()?;
    assert_eq!(cached_pack_names(&repo_cache)?, real_packs);

    plant_cached_pack(&repo_cache, STALE_PACK_FULL_LOAD)?;
    assert!(
        cached_pack_names(&repo_cache)?
            .iter()
            .any(|name| name == STALE_PACK_FULL_LOAD)
    );

    let _repo = repo.to_indexed()?;
    assert_eq!(cached_pack_names(&repo_cache)?, real_packs);

    Ok(())
}
