#![allow(missing_docs)]
use std::{fs::File, io::Read, sync::Arc};

use anyhow::Result;
use rstest::rstest;
use rustic_core::{
    Credentials, FileType, Id, Repository, RepositoryBackends, RepositoryOptions, WriteBackend,
};
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
    let options = RepositoryOptions::default();
    let credentials = Credentials::password(password);
    let repo = Repository::new(&options, &be)?;
    if should_work {
        assert!(repo.open(&credentials).is_ok());
    } else {
        assert!(
            repo.open(&credentials)
                .is_err_and(|err| err.is_incorrect_password())
        );
    }
    Ok(())
}

#[test]
// using an invalid keyfile: Here the data in the keyfile is not valid
fn test_keys_failing_passes() -> Result<()> {
    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    add_to_be(&be, FileType::Key, "tests/fixtures/key-failing")?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default();
    let repo = Repository::new(&options, &be)?;
    assert!(
        repo.open(&Credentials::password("test"))
            .is_err_and(|err| !err.is_incorrect_password())
    );
    Ok(())
}

#[rstest]
#[case::zero_n(0, 8, 6)]
#[case::excessive_memory(1 << 31, 8, 1)]
#[case::excessive_work(1 << 15, 8, 1 << 20)]
// using a keyfile with invalid or excessive scrypt parameters: The key must be rejected
// before running scrypt instead of panicking, aborting on allocation failure or running for hours
fn test_keys_with_excessive_scrypt_params_fail(
    #[case] n: u32,
    #[case] r: u32,
    #[case] p: u32,
) -> Result<()> {
    let mut key: serde_json::Value =
        serde_json::from_slice(&std::fs::read("tests/fixtures/key1")?)?;
    key["N"] = n.into();
    key["r"] = r.into();
    key["p"] = p.into();

    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    _ = add_bytes_to_be(&be, FileType::Key, serde_json::to_vec(&key)?)?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default();
    let repo = Repository::new(&options, &be)?;
    assert!(
        repo.open(&Credentials::password("test"))
            .is_err_and(|err| !err.is_incorrect_password())
    );
    Ok(())
}

fn truncated_key_file() -> Result<Vec<u8>> {
    let mut key: serde_json::Value =
        serde_json::from_slice(&std::fs::read("tests/fixtures/key1")?)?;
    key["data"] = "AAAAAAAAAAAAAAAAAAAAAAAAAAA=".into();
    Ok(serde_json::to_vec(&key)?)
}

#[rstest]
#[case::correct_password("test")]
#[case::wrong_password("wrong")]
fn test_only_key_file_too_short_is_reported_as_damaged(#[case] password: &str) -> Result<()> {
    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    _ = add_bytes_to_be(&be, FileType::Key, truncated_key_file()?)?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let repo = Repository::new(&RepositoryOptions::default(), &be)?;
    let err = repo.open(&Credentials::password(password)).unwrap_err();
    assert!(err.is_code("C004"), "{err}");
    assert!(!err.is_incorrect_password());
    Ok(())
}

#[rstest]
#[case::matching_key("test", true)]
#[case::no_matching_key("wrong", false)]
fn test_key_file_too_short_beside_a_valid_key(
    #[case] password: &str,
    #[case] should_open: bool,
) -> Result<()> {
    let be = InMemoryBackend::new();
    add_to_be(&be, FileType::Config, "tests/fixtures/config")?;
    add_to_be(&be, FileType::Key, "tests/fixtures/key1")?;
    let damaged_id = add_bytes_to_be(&be, FileType::Key, truncated_key_file()?)?;

    let be = RepositoryBackends::new(Arc::new(be), None);
    let repo = Repository::new(&RepositoryOptions::default(), &be)?;
    match repo.open(&Credentials::password(password)) {
        Ok(_) => assert!(should_open),
        Err(err) => {
            assert!(!should_open, "{err}");
            assert!(err.is_incorrect_password());
            assert_eq!(
                err.context_value("damaged_key_files"),
                Some(damaged_id.to_string().as_str())
            );
        }
    }
    Ok(())
}

fn add_to_be(be: &impl WriteBackend, tpe: FileType, file: &str) -> Result<()> {
    let mut bytes = Vec::new();
    _ = File::open(file)?.read_to_end(&mut bytes)?;
    _ = add_bytes_to_be(be, tpe, bytes)?;
    Ok(())
}

fn add_bytes_to_be(be: &impl WriteBackend, tpe: FileType, bytes: Vec<u8>) -> Result<Id> {
    let id = Id::new(Sha256::digest(&bytes).into());
    be.write_bytes(tpe, &id, true, bytes.into())?;
    Ok(id)
}
