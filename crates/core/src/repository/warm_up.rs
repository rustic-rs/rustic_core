use std::process::Command;
use std::thread::sleep;

use log::{debug, error, warn};
use rayon::ThreadPoolBuilder;

use crate::{
    backend::{FileType, ReadBackend},
    error::{ErrorKind, RusticError, RusticResult},
    progress::{Progress, ProgressBars},
    repofile::packfile::PackId,
    repository::Repository,
    CommandInput,
};

pub(super) mod constants {
    /// The maximum number of reader threads to use for warm-up.
    pub(super) const MAX_READER_THREADS_NUM: usize = 20;
}

/// Warm up the repository and wait.
///
/// # Arguments
///
/// * `repo` - The repository to warm up.
/// * `packs` - The packs to warm up.
///
/// # Errors
///
/// * If the command could not be parsed.
/// * If the thread pool could not be created.
pub(crate) fn warm_up_wait<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = PackId> + Clone,
) -> RusticResult<()> {
    warm_up(repo, packs.clone())?;

    if let Some(warm_up_wait_cmd) = &repo.opts.warm_up_wait_command {
        warm_up_command(packs, warm_up_wait_cmd, &repo.pb, &WarmUpType::WaitPack)?;
    } else if let Some(wait) = repo.opts.warm_up_wait {
        let p = repo.pb.progress_spinner(format!("waiting {wait}..."));
        sleep(*wait);
        p.finish();
    }
    Ok(())
}

/// Warm up the repository.
///
/// # Arguments
///
/// * `repo` - The repository to warm up.
/// * `packs` - The packs to warm up.
///
/// # Errors
///
/// * If the command could not be parsed.
/// * If the thread pool could not be created.
pub(crate) fn warm_up<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = PackId>,
) -> RusticResult<()> {
    if let Some(warm_up_cmd) = &repo.opts.warm_up_command {
        warm_up_command(packs, warm_up_cmd, &repo.pb, &WarmUpType::WarmUp)?;
    } else if repo.be.needs_warm_up() {
        warm_up_repo(repo, packs)?;
    }
    Ok(())
}

#[derive(Debug)]
enum WarmUpType {
    WarmUp,
    WaitPack,
}

/// Warm up the repository using a command.
///
/// # Arguments
///
/// * `packs` - The packs to warm up.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_command<P: ProgressBars>(
    packs: impl ExactSizeIterator<Item = PackId>,
    command: &CommandInput,
    pb: &P,
    ty: &WarmUpType,
) -> RusticResult<()> {
    let p = pb.progress_counter(match ty {
        WarmUpType::WarmUp => "warming up packs...",
        WarmUpType::WaitPack => "waiting for packs to be ready...",
    });
    p.set_length(packs.len() as u64);
    for pack in packs {
        let args: Vec<_> = command
            .args()
            .iter()
            .map(|c| c.replace("%id", &pack.to_hex()))
            .collect();

        debug!("calling {command:?}...");

        let status = Command::new(command.command())
            .args(&args)
            .status()
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::ExternalCommand,
                    "Error in executing warm-up command `{command}`.",
                    err,
                )
                .attach_context("command", command.to_string())
                .attach_context("type", format!("{ty:?}"))
            })?;

        if !status.success() {
            warn!("{ty:?} command was not successful for pack {pack:?}. {status}");
        }
        p.inc(1);
    }
    p.finish();
    Ok(())
}

/// Warm up the repository.
///
/// # Arguments
///
/// * `repo` - The repository to warm up.
/// * `packs` - The packs to warm up.
///
/// # Errors
///
/// * If the thread pool could not be created.
fn warm_up_repo<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = PackId>,
) -> RusticResult<()> {
    let progress_bar = repo.pb.progress_counter("warming up packs...");
    progress_bar.set_length(packs.len() as u64);

    let pool = ThreadPoolBuilder::new()
        .num_threads(constants::MAX_READER_THREADS_NUM)
        .build()
        .map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to create thread pool for warm-up. Please try again.",
                err,
            )
        })?;
    let progress_bar_ref = &progress_bar;
    let backend = &repo.be;
    pool.in_place_scope(|scope| {
        for pack in packs {
            scope.spawn(move |_| {
                if let Err(err) = backend.warm_up(FileType::Pack, &pack) {
                    // FIXME: Use error handling
                    error!("warm-up failed for pack {pack:?}. {}", err.display_log());
                };
                progress_bar_ref.inc(1);
            });
        }
    });

    progress_bar_ref.finish();

    Ok(())
}
