//! `key` example
use rustic_backend::BackendOptions;
use rustic_core::{KeyOptions, Repository, RepositoryOptions};
use simplelog::{Config, LevelFilter, SimpleLogger};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Display info logs
    let _ = SimpleLogger::init(LevelFilter::Info, Config::default());

    // Initialize Backends
    let backends = BackendOptions::default()
        .repository("/tmp/repo")
        .to_backends()?;

    // Open repository
    let repo_opts = RepositoryOptions::default().password("test");

    let repo = Repository::new(&repo_opts, &backends)?.open()?;

    // Add a new key with the given password
    let key_opts = KeyOptions::default();
    repo.add_key("new_password", &key_opts)?;
    Ok(())
}
