//! `key` subcommand
use derive_setters::Setters;

use crate::{
    backend::{FileType, WriteBackend, decrypt::DecryptWriteBackend},
    crypto::{aespoly1305::Key, hasher::hash},
    error::{ErrorKind, RusticError, RusticResult},
    repofile::{KeyFile, KeyId},
    repository::{Open, Repository},
};

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Debug, Clone, Default, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `key` command. These are used when creating a new key.
pub struct KeyOptions {
    /// Set 'hostname' in public key information
    #[cfg_attr(feature = "clap", clap(long))]
    pub hostname: Option<String>,

    /// Set 'username' in public key information
    #[cfg_attr(feature = "clap", clap(long))]
    pub username: Option<String>,

    /// Add 'created' date in public key information
    #[cfg_attr(feature = "clap", clap(long))]
    pub with_created: bool,
}

/// Add the current key to the repository.
///
/// # Type Parameters
///
/// * `P` - The progress bar type
/// * `S` - The state the repository is in
///
/// # Arguments
///
/// * `repo` - The repository to add the key to
/// * `opts` - The key options to use
/// * `pass` - The password to encrypt the key with
///
/// # Errors
///
/// * If the key could not be serialized
///
/// # Returns
///
/// The id of the key.
pub(crate) fn add_current_key_to_repo<S: Open>(
    repo: &Repository<S>,
    opts: &KeyOptions,
    pass: &str,
) -> RusticResult<KeyId> {
    let key = repo.dbe().key();
    add_key_to_repo(repo, opts, pass, *key)
}

/// Initialize a new key.
///
/// # Type Parameters
///
/// * `P` - The progress bar type
/// * `S` - The state the repository is in
///
/// # Arguments
///
/// * `repo` - The repository to add the key to
/// * `opts` - The key options to use
/// * `pass` - The password to encrypt the key with
///
/// # Returns
///
/// A tuple of the key and the id of the key.
pub(crate) fn init_key<S>(
    repo: &Repository<S>,
    opts: &KeyOptions,
    pass: &str,
) -> RusticResult<(Key, KeyId)> {
    // generate key
    let key = Key::new();
    Ok((key, add_key_to_repo(repo, opts, pass, key)?))
}

/// Add a key to the repository.
///
/// # Arguments
///
/// * `repo` - The repository to add the key to
/// * `opts` - The key options to use
/// * `pass` - The password to encrypt the key with
/// * `key` - The key to add
///
/// # Errors
///
/// * If the key could not be serialized.
///
/// # Returns
///
/// The id of the key.
pub(crate) fn add_key_to_repo<S>(
    repo: &Repository<S>,
    opts: &KeyOptions,
    pass: &str,
    key: Key,
) -> RusticResult<KeyId> {
    let ko = opts.clone();
    let keyfile = KeyFile::generate(key, &pass, ko.hostname, ko.username, ko.with_created)?;

    let data = serde_json::to_vec(&keyfile).map_err(|err| {
        RusticError::with_source(
            ErrorKind::InputOutput,
            "Failed to serialize keyfile to JSON.",
            err,
        )
    })?;

    let id = KeyId::from(hash(&data));

    repo.be
        .write_bytes(FileType::Key, &id, false, data.into())?;

    Ok(id)
}
