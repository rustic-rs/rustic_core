//! `ls` example
use globset::Glob;
use rustic_backend::BackendOptions;
use rustic_core::{FindMatches, Repository, RepositoryOptions};
use simplelog::{Config, LevelFilter, SimpleLogger};
use std::{error::Error, path::PathBuf};

// don't warn about try_from paths conversion on unix
#[allow(clippy::unnecessary_fallible_conversions)]
fn main() -> Result<(), Box<dyn Error>> {
    // Display info logs
    let _ = SimpleLogger::init(LevelFilter::Info, Config::default());

    // Initialize Backends
    let backends = BackendOptions::default()
        .repository("/tmp/repo")
        .to_backends()?;

    // Open repository
    let repo_opts = RepositoryOptions::default().password("test");

    let repo = Repository::new(&repo_opts, &backends)?
        .open()?
        .to_indexed()?;

    let mut snapshots = repo.get_all_snapshots()?;
    snapshots.sort_unstable();
    let tree_ids = snapshots.iter().map(|sn| sn.tree);

    let glob = Glob::new("*.rs")?.compile_matcher();
    let FindMatches {
        paths,
        nodes,
        matches,
    } = repo.find_matching_nodes(tree_ids, &|path, _| {
        glob.is_match(PathBuf::try_from(path).unwrap())
    })?;
    for (snap, matches) in snapshots.iter().zip(matches) {
        println!("results in {snap:?}");
        for (path_idx, node_idx) in matches {
            println!("path: {:?}, node: {:?}", paths[path_idx], nodes[node_idx]);
        }
    }

    Ok(())
}
