#![allow(missing_docs)]
use std::{fs::File, io::Read, sync::Arc};

use anyhow::Result;
use rstest::rstest;
use rustic_core::{FileType, Id, Repository, RepositoryBackends, RepositoryOptions, WriteBackend};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;
use sha2::{Digest, Sha256};

#[rstest]
#[case("test", true)]
#[case("test2", true)]
#[case("wrong", false)]
fn test_working_keys_passes(#[case] password: &str, #[case] should_work: bool) -> Result<()> {
    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    add_to_be(&be, FileType::Key, "tests/fixtures/key1")?;
    add_to_be(&be, FileType::Key, "tests/fixtures/key2")?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password(password);
    let repo = Repository::new(&options, &be)?;
    if should_work {
        assert!(repo.open().is_ok());
    } else {
        assert!(repo.open().is_err_and(|err| err.is_incorrect_password()));
    }
    Ok(())
}

#[test]
// using an invalid keyfile: Here the scrypt params are not valid
fn test_keys_failing_passes() -> Result<()> {
    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    add_to_be(&be, FileType::Key, "tests/fixtures/key-failing")?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().password("test");
    let repo = Repository::new(&options, &be)?;
    assert!(repo.open().is_err_and(|err| !err.is_incorrect_password()));
    Ok(())
}

fn add_to_be(be: &impl WriteBackend, tpe: FileType, file: &str) -> Result<()> {
    let mut bytes = Vec::new();
    _ = File::open(file)?.read_to_end(&mut bytes)?;
    let id = Id::new(Sha256::digest(&bytes).into());
    be.write_bytes(tpe, &id, true, bytes.into())?;
    Ok(())
}
