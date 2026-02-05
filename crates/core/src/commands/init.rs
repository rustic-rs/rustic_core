//! `init` subcommand

use log::info;

use crate::{
    backend::WriteBackend,
    chunker::rabin::random_poly,
    commands::{
        config::{ConfigOptions, save_config},
        key::{KeyOptions, init_key},
    },
    crypto::aespoly1305::Key,
    error::RusticResult,
    id::Id,
    repofile::{ConfigFile, KeyId, configfile::RepositoryId},
    repository::{Repository, credentials::Credentials},
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
/// * `credentials` - The credentials to use.
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
pub(crate) fn init<S>(
    repo: &Repository<S>,
    credentials: &Credentials,
    key_opts: &KeyOptions,
    config_opts: &ConfigOptions,
) -> RusticResult<(Key, Option<KeyId>, ConfigFile)> {
    // Create config first to allow catching errors from here without writing anything
    let repo_id = RepositoryId::from(Id::random());
    let chunker_poly = random_poly()?;
    let mut config = ConfigFile::new(2, repo_id, chunker_poly);
    if repo.be_hot.is_some() {
        // for hot/cold repository, `config` must be identical to the config file which is read by the backend, i.e. the one saved in the hot repo.
        // Note: init_with_config does handle the is_hot config correctly for the hot and the cold repo.
        config.is_hot = Some(true);
    }
    config_opts.apply(&mut config)?;

    let (key, key_id) = init_with_config(repo, credentials, key_opts, &config)?;
    info!("repository {repo_id} successfully created.");

    Ok((key, key_id, config))
}

/// Initialize a new repository with a given config.
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
/// * `config` - The config to use.
///
/// # Returns
///
/// The key used to encrypt the config.
pub(crate) fn init_with_config<S>(
    repo: &Repository<S>,
    credentials: &Credentials,
    key_opts: &KeyOptions,
    config: &ConfigFile,
) -> RusticResult<(Key, Option<KeyId>)> {
    repo.be.create()?;
    let (key, id) = match credentials {
        Credentials::Password(password) => {
            let (key, id) = init_key(repo, key_opts, password)?;
            info!("key {id} successfully added.");
            (key, Some(id))
        }
        Credentials::Masterkey(key) => (key.key(), None),
    };
    save_config(repo, config.clone(), key)?;

    Ok((key, id))
}
