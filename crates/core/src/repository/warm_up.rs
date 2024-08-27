use std::process::Command;
use std::thread::sleep;

use log::{debug, error, warn};
use rayon::ThreadPoolBuilder;

use crate::{
    backend::{FileType, ReadBackend},
    error::{RepositoryErrorKind, RusticResult},
    id::Id,
    progress::{Progress, ProgressBars},
    repository::Repository,
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
/// * [`RepositoryErrorKind::FromSplitError`] - If the command could not be parsed.
/// * [`RepositoryErrorKind::FromThreadPoolbilderError`] - If the thread pool could not be created.
///
/// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
/// [`RepositoryErrorKind::FromThreadPoolbilderError`]: crate::error::RepositoryErrorKind::FromThreadPoolbilderError
pub(crate) fn warm_up_wait<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = Id>,
) -> RusticResult<()> {
    warm_up(repo, packs)?;
    if let Some(wait) = repo.opts.warm_up_wait {
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
/// * [`RepositoryErrorKind::FromSplitError`] - If the command could not be parsed.
/// * [`RepositoryErrorKind::FromThreadPoolbilderError`] - If the thread pool could not be created.
///
/// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
/// [`RepositoryErrorKind::FromThreadPoolbilderError`]: crate::error::RepositoryErrorKind::FromThreadPoolbilderError
pub(crate) fn warm_up<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = Id>,
) -> RusticResult<()> {
    if !repo.opts.warm_up_command.is_empty() {
        warm_up_command(packs, &repo.opts.warm_up_command, &repo.pb)?;
    } else if repo.be.needs_warm_up() {
        warm_up_repo(repo, packs)?;
    }
    Ok(())
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
/// * [`RepositoryErrorKind::FromSplitError`] - If the command could not be parsed.
///
/// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
fn warm_up_command<P: ProgressBars>(
    packs: impl ExactSizeIterator<Item = Id>,
    command: &[String],
    pb: &P,
) -> RusticResult<()> {
    let p = pb.progress_counter("warming up packs...");
    p.set_length(packs.len() as u64);
    for pack in packs {
        let command: Vec<_> = command
            .iter()
            .map(|c| c.replace("%id", &pack.to_hex()))
            .collect();
        debug!("calling {command:?}...");
        let status = Command::new(&command[0]).args(&command[1..]).status()?;
        if !status.success() {
            warn!("warm-up command was not successful for pack {pack:?}. {status}");
        }
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
/// * [`RepositoryErrorKind::FromThreadPoolbilderError`] - If the thread pool could not be created.
///
/// [`RepositoryErrorKind::FromThreadPoolbilderError`]: crate::error::RepositoryErrorKind::FromThreadPoolbilderError
fn warm_up_repo<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    packs: impl ExactSizeIterator<Item = Id>,
) -> RusticResult<()> {
    let progress_bar = repo.pb.progress_counter("warming up packs...");
    progress_bar.set_length(packs.len() as u64);

    let pool = ThreadPoolBuilder::new()
        .num_threads(constants::MAX_READER_THREADS_NUM)
        .build()
        .map_err(RepositoryErrorKind::FromThreadPoolbilderError)?;
    let progress_bar_ref = &progress_bar;
    let backend = &repo.be;
    pool.in_place_scope(|scope| {
        for pack in packs {
            scope.spawn(move |_| {
                if let Err(e) = backend.warm_up(FileType::Pack, &pack) {
                    // FIXME: Use error handling
                    error!("warm-up failed for pack {pack:?}. {e}");
                };
                progress_bar_ref.inc(1);
            });
        }
    });

    progress_bar_ref.finish();

    Ok(())
}
