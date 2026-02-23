use std::process::Command;
use std::thread::sleep;

use itertools::Itertools;
use log::{debug, error, warn};
use rayon::ThreadPoolBuilder;

use crate::{
    CommandInput, Id, Progress,
    backend::{FileType, ReadBackend},
    error::{ErrorKind, RusticError, RusticResult},
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
/// * `tpe` - The filetype of the ids.
/// * `ids` - The ids to warm up.
///
/// # Errors
///
/// * If the command could not be parsed.
/// * If the thread pool could not be created.
pub(crate) fn warm_up_wait<S>(
    repo: &Repository<S>,
    tpe: FileType,
    ids: impl ExactSizeIterator<Item = Id> + Clone,
) -> RusticResult<()> {
    if ids.len() > 0 {
        warm_up(repo, tpe, ids.clone())?;

        if let Some(warm_up_wait_cmd) = &repo.opts.warm_up_wait_command {
            warm_up_command(
                tpe,
                ids,
                warm_up_wait_cmd,
                repo,
                &WarmUpType::Wait,
                repo.opts.warm_up_batch.unwrap_or(1),
                &repo.be,
            )?;
        } else if let Some(wait) = repo.opts.warm_up_wait {
            let p = repo.progress_spinner(&format!("waiting {wait}..."));
            sleep(
                wait.try_into()
                    // ignore conversation errors, but print out warning
                    .inspect_err(|err| warn!("cannot wait for warm-up: {err}"))
                    .unwrap_or_default(),
            );
            p.finish();
        }
    }
    Ok(())
}

/// Warm up the repository.
///
/// # Arguments
///
/// * `repo` - The repository to warm up.
/// * `tpe` - The filetype of the ids.
/// * `ids` - The ids to warm up.
///
/// # Errors
///
/// * If the command could not be parsed.
/// * If the thread pool could not be created.
pub(crate) fn warm_up<S>(
    repo: &Repository<S>,
    tpe: FileType,
    ids: impl ExactSizeIterator<Item = Id>,
) -> RusticResult<()> {
    if ids.len() > 0 {
        if let Some(warm_up_cmd) = &repo.opts.warm_up_command {
            warm_up_command(
                tpe,
                ids,
                warm_up_cmd,
                repo,
                &WarmUpType::WarmUp,
                repo.opts.warm_up_batch.unwrap_or(1),
                &repo.be,
            )?;
        } else if repo.be.needs_warm_up() {
            warm_up_repo(repo, tpe, ids)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
enum WarmUpType {
    WarmUp,
    Wait,
}

/// Warm up the repository using a command.
///
/// # Arguments
///
/// * `tpe` - The filetype of the ids.
/// * `ids` - The ids to warm up.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `batch_size` - The number of ids to process in each batch.
/// * `backend` - The backend to get id paths from.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_command<S>(
    tpe: FileType,
    ids: impl ExactSizeIterator<Item = Id>,
    command: &CommandInput,
    repo: &Repository<S>,
    ty: &WarmUpType,
    batch_size: usize,
    backend: &impl ReadBackend,
) -> RusticResult<()> {
    let use_plural = command.uses_plural_placeholders()?;

    let total = ids.len();

    let p = repo.progress_counter(&match ty {
        WarmUpType::WarmUp => format!("warming up {tpe}(s)..."),
        WarmUpType::Wait => format!("waiting for {tpe}(s) to be ready..."),
    });
    p.set_length(total as u64);

    let chunks = ids.chunks(batch_size);
    for batch in &chunks {
        let batch: Vec<_> = batch.collect();
        if use_plural {
            warm_up_batch_plural(tpe, &batch, command, ty, backend, &p)?;
        } else {
            warm_up_batch_singular(tpe, &batch, command, ty, backend, &p)?;
        }
    }

    p.finish();
    Ok(())
}

/// Warm up a batch of ids using singular mode (one command per id).
///
/// # Arguments
///
/// * `tpe` - The filetype of the ids.
/// * `batch` - The ids in this batch.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `backend` - The backend to get id paths from.
/// * `progress` - The progress bar to update.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_batch_singular(
    tpe: FileType,
    batch: &[Id],
    command: &CommandInput,
    ty: &WarmUpType,
    backend: &impl ReadBackend,
    progress: &Progress,
) -> RusticResult<()> {
    let file_type = tpe.to_string();
    let children: Vec<_> = batch
        .iter()
        .map(|id| {
            let path = backend.warmup_path(tpe, id);
            let id = id.to_hex().to_string();

            let args: Vec<_> = command
                .args()
                .iter()
                .map(|c| {
                    c.replace("%tpe", &file_type)
                        .replace("%id", &id)
                        .replace("%path", &path)
                })
                .collect();

            debug!("spawning {command:?} for id {id:?}...");

            let child = Command::new(command.command())
                .args(&args)
                .spawn()
                .map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::ExternalCommand,
                        "Error in spawning warm-up command `{command}`.",
                        err,
                    )
                    .attach_context("command", command.to_string())
                    .attach_context("id", &id)
                    .attach_context("type", format!("{ty:?}"))
                })?;

            Ok((child, id))
        })
        .collect::<RusticResult<Vec<_>>>()?;

    let mut failed_ids = Vec::new();

    for (mut child, id) in children {
        debug!("waiting for warm-up command for id {id}...");

        let status = child.wait().map_err(|err| {
            RusticError::with_source(
                ErrorKind::ExternalCommand,
                "Error waiting for warm-up command `{command}`.",
                err,
            )
            .attach_context("command", command.to_string())
            .attach_context("id", &id)
            .attach_context("type", format!("{ty:?}"))
        })?;

        if !status.success() {
            failed_ids.push((id, status));
        }

        progress.inc(1);
    }

    if !failed_ids.is_empty() {
        let error_msg = format!(
            "{ty:?} command failed for {}/{} id(s): {}",
            failed_ids.len(),
            batch.len(),
            failed_ids
                .iter()
                .map(|(id, status)| format!("{id:?} ({status})"))
                .collect::<Vec<_>>()
                .join(", ")
        );

        return Err(RusticError::new(ErrorKind::ExternalCommand, error_msg)
            .attach_context("command", command.to_string())
            .attach_context("failed_ids", failed_ids.len().to_string())
            .attach_context("total_ids", batch.len().to_string())
            .attach_context("type", format!("{ty:?}")));
    }

    Ok(())
}

/// Warm up a batch of ids using plural mode (single command with all values).
///
/// # Arguments
///
/// * `tpe` - The filetype of the ids.
/// * `batch` - The ids in this batch.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `backend` - The backend to get id paths from.
/// * `progress` - The progress bar to update.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_batch_plural(
    tpe: FileType,
    batch: &[Id],
    command: &CommandInput,
    ty: &WarmUpType,
    backend: &impl ReadBackend,
    progress: &Progress,
) -> RusticResult<()> {
    let file_type = tpe.to_string();
    let cmd_str = command.to_string();
    let use_ids = cmd_str.contains("%ids");
    let use_paths = cmd_str.contains("%paths");

    let mut args = Vec::new();

    for arg in command.args() {
        if use_ids && arg.contains("%ids") {
            args.extend(batch.iter().map(|id| id.to_hex().to_string()));
        } else if use_paths && arg.contains("%paths") {
            args.extend(
                batch
                    .iter()
                    .map(|id| backend.warmup_path(FileType::Pack, id)),
            );
        } else {
            args.push(arg.replace("%tpe", &file_type));
        }
    }

    debug!("calling {command:?} with {} id(s)...", batch.len());

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
        return Err(RusticError::new(
            ErrorKind::ExternalCommand,
            format!(
                "{ty:?} command failed for batch of {} id(s). {status}",
                batch.len()
            ),
        )
        .attach_context("command", command.to_string())
        .attach_context("batch_size", batch.len().to_string())
        .attach_context("status", status.to_string())
        .attach_context("type", format!("{ty:?}")));
    }

    progress.inc(batch.len() as u64);
    Ok(())
}

/// Warm up the repository.
///
/// # Arguments
///
/// * `repo` - The repository to warm up.
/// * `tpe` - The filetype of the ids
/// * `ids` - The ids to warm up.
///
/// # Errors
///
/// * If the thread pool could not be created.
fn warm_up_repo<S>(
    repo: &Repository<S>,
    tpe: FileType,
    ids: impl ExactSizeIterator<Item = Id>,
) -> RusticResult<()> {
    let progress_bar = repo.progress_counter("warming up {tpe}(s)...");
    progress_bar.set_length(ids.len() as u64);

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
        for id in ids {
            scope.spawn(move |_| {
                if let Err(err) = backend.warm_up(tpe, &id) {
                    // FIXME: Use error handling
                    error!("warm-up failed for id {id:?}. {}", err.display_log());
                }
                progress_bar_ref.inc(1);
            });
        }
    });

    progress_bar_ref.finish();

    Ok(())
}
