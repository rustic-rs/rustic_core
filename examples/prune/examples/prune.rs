//! `prune` example
use rustic_backend::BackendOptions;
use rustic_core::{PruneOptions, Repository, RepositoryOptions};
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

    let repo = Repository::new(&repo_opts, backends)?.open()?;

    let prune_opts = PruneOptions::default();
    let prune_plan = repo.prune_plan(&prune_opts)?;
    println!("{:?}", prune_plan.stats);
    println!("to repack: {:?}", prune_plan.repack_packs());
    // to run the plan uncomment this line:
    // prune_plan.do_prune(&repo, &prune_opts)?;
    Ok(())
}
