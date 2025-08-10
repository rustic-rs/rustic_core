//! `init` example
use rustic_backend::BackendOptions;
use rustic_core::{ConfigOptions, KeyOptions, Repository, RepositoryOptions};
use simplelog::{Config, LevelFilter, SimpleLogger};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Display info logs
    let _ = SimpleLogger::init(LevelFilter::Info, Config::default());

    // Initialize Backends
    let backends = BackendOptions::default()
        .repository("/tmp/repo")
        .to_backends()?;

    // Init repository
    let repo_opts = RepositoryOptions::default().password("test");
    let key_opts = KeyOptions::default();
    let config_opts = ConfigOptions::default();
    let _repo = Repository::new(&repo_opts, &backends)?.init(&key_opts, &config_opts)?;

    // -> use _repo for any operation on an open repository
    Ok(())
}
