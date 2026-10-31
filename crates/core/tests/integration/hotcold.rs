use std::{path::PathBuf, str::FromStr, sync::Arc};

use anyhow::Result;
use pretty_assertions::assert_eq;
use rstest::rstest;

use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, Credentials, FileType, Id, KeyOptions,
    ReadBackend, Repository, RepositoryBackends, RepositoryOptions, WriteBackend,
    repofile::SnapshotFile,
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

use super::{TestSource, tar_gz_testdata};

#[rstest]
fn hot_cold(tar_gz_testdata: Result<TestSource>) -> Result<()> {
    // Fixtures
    let source = tar_gz_testdata?;

    let be_hot = InMemoryBackend::new();
    let be_cold = InMemoryBackend::new_cold();
    let be_cold = Arc::new(be_cold);
    let be = RepositoryBackends::new(be_cold.clone(), Some(Arc::new(be_hot)));
    let options = RepositoryOptions::default();
    let repo = Repository::new(&options, &be)?;
    let key_opts = KeyOptions::default();
    let config_opts = &ConfigOptions::default();
    let creds = Credentials::password("test");
    let repo = repo
        .init(&creds, &key_opts, config_opts)?
        .to_indexed_ids()?;

    let paths = &source.path_list();

    // we use as_path to not depend on the actual tempdir
    let opts = BackupOptions::default().as_path(PathBuf::from_str("test")?);

    // backup
    let snapshot = repo.backup(&opts, paths, SnapshotFile::default())?;

    // get all snapshots and check them
    let all_snapshots = repo.get_all_snapshots()?;
    assert_eq!(vec![snapshot], all_snapshots);
    repo.check(CheckOptions::default().read_data(true))?
        .is_ok()?;

    // remove keys, config and index files from hot repository
    for tpe in [FileType::Key, FileType::Config, FileType::Index] {
        for id in be.repo_hot().unwrap().list(tpe)? {
            be.repo_hot().unwrap().remove(tpe, &id, false)?;
        }
    }
    assert!(repo.check(CheckOptions::default())?.is_ok().is_err());

    // repo cannot be opened normally
    let repo = Repository::new(&options, &be)?;
    assert!(repo.open(&creds).is_err());

    // but with open_with_password_only_cold
    let repo = Repository::new(&options, &be)?;
    let repo = repo.open_only_cold(&Credentials::password("test"))?;

    // repair repository
    repo.init_hot()?;
    repo.repair_hotcold_except_packs(false)?;

    // now we should be able to open the repository again.
    let repo = Repository::new(&options, &be)?.open(&creds)?;

    // remove pack files from hot repository
    for id in be.repo_hot().unwrap().list(FileType::Pack)? {
        be.repo_hot().unwrap().remove(FileType::Pack, &id, true)?;
    }
    assert!(repo.check(CheckOptions::default())?.is_ok().is_err());

    // repair
    repo.repair_hotcold_packs(false)?;
    repo.check(CheckOptions::default())?.is_ok()?;

    // remove index files from cold repository
    for id in be_cold.list(FileType::Index)? {
        be_cold.remove(FileType::Index, &id, true)?;
    }
    assert!(repo.check(CheckOptions::default())?.is_ok().is_err());

    // repair
    repo.repair_hotcold_except_packs(false)?;
    repo.check(CheckOptions::default())?.is_ok()?;

    // remove tree pack files from cold repository
    for id in be.repo_hot().unwrap().list(FileType::Pack)? {
        be_cold.remove(FileType::Pack, &id, true)?;
    }
    assert!(repo.check(CheckOptions::default())?.is_ok().is_err());

    // repair
    repo.repair_hotcold_packs(false)?;
    repo.check(CheckOptions::default())?.is_ok()?;
    Ok(())
}

fn backends(hot_cold: bool) -> RepositoryBackends {
    if hot_cold {
        RepositoryBackends::new(
            Arc::new(InMemoryBackend::new_cold()),
            Some(Arc::new(InMemoryBackend::new())),
        )
    } else {
        RepositoryBackends::new(Arc::new(InMemoryBackend::new()), None)
    }
}

#[rstest]
fn init_with_config_from_other_repo(
    #[values(false, true)] source_hot_cold: bool,
    #[values(false, true)] target_hot_cold: bool,
) -> Result<()> {
    let options = RepositoryOptions::default();
    let key_opts = KeyOptions::default();
    let creds = Credentials::password("test");
    let source = Repository::new(&options, &backends(source_hot_cold))?.init(
        &creds,
        &key_opts,
        &ConfigOptions::default(),
    )?;

    // initialize the target with the config of the source, like `copy --init` does
    let mut config = source.config().clone();
    config.id = Id::random().into();
    let be = backends(target_hot_cold);
    let target = Repository::new(&options, &be)?.init_with_config(&creds, &key_opts, config)?;
    target.check(CheckOptions::default())?.is_ok()?;

    // the target can be opened again
    let target = Repository::new(&options, &be)?.open(&creds)?;
    target.check(CheckOptions::default())?.is_ok()?;
    Ok(())
}
