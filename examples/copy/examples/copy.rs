//! `copy` example
use rustic_backend::BackendOptions;
use rustic_core::{CopySnapshot, Repository, RepositoryOptions};
use simplelog::{Config, LevelFilter, SimpleLogger};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Display info logs
    let _ = SimpleLogger::init(LevelFilter::Info, Config::default());

    // Initialize src backends
    let src_backends = BackendOptions::default()
        .repository("/tmp/repo")
        .to_backends()?;

    // Open repository
    let src_repo_opts = RepositoryOptions::default().password("test");

    let src_repo = Repository::new(&src_repo_opts, &src_backends)?
        .open()?
        .to_indexed()?;

    // Initialize dst backends
    let dst_backends = BackendOptions::default()
        .repository("/tmp/repo")
        .to_backends()?;

    let dst_repo_opts = RepositoryOptions::default().password("test");

    let dst_repo = Repository::new(&dst_repo_opts, &dst_backends)?
        .open()?
        .to_indexed_ids()?;

    // get snapshots which are missing in dst_repo
    let snapshots = src_repo.get_all_snapshots()?;
    let snaps = dst_repo.relevant_copy_snapshots(|_| true, &snapshots)?;

    // copy only relevant snapshots
    src_repo.copy(
        &dst_repo,
        snaps
            .iter()
            .filter_map(|CopySnapshot { relevant, sn }| relevant.then_some(sn)),
    )?;
    Ok(())
}
