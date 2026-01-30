//! `forget` example
use rustic_backend::BackendOptions;
use rustic_core::{
    Credentials, KeepOptions, Repository, RepositoryOptions, SnapshotGroupCriterion,
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

    let repo = Repository::new(&repo_opts, &backends)?.open(&Credentials::password("test"))?;

    // Check repository with standard options
    let group_by = SnapshotGroupCriterion::default();
    let keep = KeepOptions::default().keep_daily(5).keep_weekly(10);
    let snaps = repo.get_forget_snapshots(&keep, group_by, |_| true)?;
    println!("{snaps:?}");
    // to remove the snapshots-to-forget, uncomment this line:
    // repo.delete_snapshots(&snaps.into_forget_ids())?
    Ok(())
}
