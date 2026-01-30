//! `config` example
use rustic_backend::BackendOptions;
use rustic_core::{
    max_compression_level, ConfigOptions, Credentials, Repository, RepositoryOptions,
};
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
    let repo_opts = RepositoryOptions::default();
    let mut repo = Repository::new(&repo_opts, &backends)?.open(&Credentials::password("test"))?;

    // Set Config, e.g. Compression level
    let config_opts = ConfigOptions::default().set_compression(max_compression_level());
    repo.apply_config(&config_opts)?;
    Ok(())
}
