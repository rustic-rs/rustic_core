//! `forget` example
use jiff::Zoned;
use rustic_backend::BackendOptions;
use rustic_core::{
    Credentials, ForgetGroups, Grouped, KeepOptions, Repository, RepositoryOptions,
    SnapshotGroupCriterion,
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
    let snaps = repo.get_all_snapshots()?;
    let grouped = Grouped::from_items(snaps, group_by);
    let forget_snaps =
        ForgetGroups::from_grouped_snapshots_with_retention(grouped, &keep, &Zoned::now())?;
    println!("{forget_snaps:?}");
    // to remove the snapshots-to-forget, uncomment this line:
    // repo.delete_snapshots(&forget_snaps.into_forget_ids())?
    Ok(())
}
