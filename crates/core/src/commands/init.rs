//! `init` subcommand

use log::info;

use crate::{
    backend::WriteBackend,
    chunker::random_poly,
    commands::{
        config::{save_config, ConfigOptions},
        key::{init_key, KeyOptions},
    },
    crypto::aespoly1305::Key,
    error::RusticResult,
    id::Id,
    repofile::{configfile::RepositoryId, ConfigFile},
    repository::Repository,
};

/// Initialize a new repository.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to initialize.
/// * `pass` - The password to encrypt the key with.
/// * `key_opts` - The options to create the key with.
/// * `config_opts` - The options to create the config with.
///
/// # Errors
///
/// * If no polynomial could be found in one million tries.
///
/// # Returns
///
/// A tuple of the key and the config file.
pub(crate) fn init<P, S>(
    repo: &Repository<P, S>,
    pass: &str,
    key_opts: &KeyOptions,
    config_opts: &ConfigOptions,
) -> RusticResult<(Key, ConfigFile)> {
    // Create config first to allow catching errors from here without writing anything
    let repo_id = RepositoryId::from(Id::random());
    let chunker_poly = random_poly()?;
    let mut config = ConfigFile::new(2, repo_id, chunker_poly);
    config_opts.apply(&mut config)?;

    let key = init_with_config(repo, pass, key_opts, &config)?;
    info!("repository {repo_id} successfully created.");

    Ok((key, config))
}

/// Initialize a new repository using a given [`ConfigFile`]
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to initialize.
/// * `pass` - The password to encrypt the key with.
/// * `key_opts` - The options to create the key with.
/// * `config` - The config to use
///
/// # Errors
///
/// * If the file could not be serialized to json.
///
/// # Returns
///
/// The key used to encrypt the config.
pub(crate) fn init_with_config<P, S>(
    repo: &Repository<P, S>,
    pass: &str,
    key_opts: &KeyOptions,
    config: &ConfigFile,
) -> RusticResult<Key> {
    repo.be.create()?;
    let (key, id) = init_key(repo, key_opts, pass)?;
    info!("key {id} successfully added.");
    save_config(repo, config.clone(), key)?;

    Ok(key)
}
