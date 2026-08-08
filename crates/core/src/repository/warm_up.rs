use std::io::{self, BufRead, BufReader};
use std::process::{Command, Stdio};
use std::thread::{self, sleep};

use backon::{BlockingRetryable, ExponentialBuilder};

use itertools::Itertools;
use log::{debug, error, info, warn};
use rayon::ThreadPoolBuilder;
use serde::Deserialize;

use crate::{
    CommandInput, Id, Progress,
    backend::{FileType, ReadBackend},
    error::{ErrorKind, RusticError, RusticResult},
    repository::Repository,
};

pub(super) mod constants {
    use std::time::Duration;

    /// The maximum number of reader threads to use for warm-up.
    pub(super) const MAX_READER_THREADS_NUM: usize = 20;

    /// The maximum number of retries for spawning commands.
    pub(crate) const MAX_RETRIES: usize = 5;

    /// Initial delay for exponential backoff for spawning commands.
    pub(crate) const INITIAL_DELAY: Duration = Duration::from_millis(10);
}

const PACK_PROGRESS_TYPE: &str = "pack-progress";

/// A progress report emitted by a warm-up command on stdout.
///
/// The command must output JSON Lines. For now only `type: "pack-progress"` is supported.
#[derive(Debug, Deserialize)]
struct PackProgress {
    /// The message type. Must be `"pack-progress"`.
    #[serde(rename = "type")]
    ty: String,

    /// The number of packs the command reports as warm within this invocation.
    warm: u64,
}

/// Read JSON Lines progress reports from the warm-up command's stdout.
///
/// Increments the shared progress bar by the delta between the last reported value and the new
/// one. Reports are ignored if they are not of type `pack-progress` or if `warm` is not larger than
/// the current value (progress is strictly increasing).
///
/// The `invocation_size` is used to clamp the reported value; a command must never report more
/// packs than it received.
///
/// Returns the final `warm` value reported by the command, or `0` if it emitted no protocol lines.
fn read_progress_output<R: io::Read>(reader: R, invocation_size: u64, progress: &Progress) -> u64 {
    let reader = BufReader::new(reader);
    let mut current: u64 = 0;

    for line in reader.lines().map_while(Result::ok) {
        let report: PackProgress = if let Ok(report) = serde_json::from_str(&line) {
            report
        } else {
            // For debugging/auditing purposes, log non-JSON stdout lines at info level.
            // Empty lines are ignored.
            if !line.is_empty() {
                info!("[warmup] {line}");
            }
            continue;
        };

        if report.ty != PACK_PROGRESS_TYPE {
            continue;
        }

        let warm = report.warm.min(invocation_size);
        if warm > current {
            progress.inc(warm - current);
            current = warm;
        }
    }

    current
}

/// On successful completion of a warm-up invocation, advance the progress bar by the packs that
/// were not already reported via the protocol.
fn finalize_progress(current: u64, invocation_size: u64, progress: &Progress) {
    if current < invocation_size {
        progress.inc(invocation_size - current);
    }
}

/// Configuration for retrying executing a command that the operating system reports is busy.
/// We believe this is a transient race condition that happens during unit tests when a program
/// is created and then immediately executed.
fn execute_cmd_retry() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(constants::INITIAL_DELAY)
        .with_max_times(constants::MAX_RETRIES)
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

    // Spawn a reader for each child's stdout while the commands run concurrently.
    let readers: Vec<_> = batch
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

            let mut child = (|| {
                Command::new(command.command())
                    .args(&args)
                    .stdout(Stdio::piped())
                    .spawn()
            })
            .retry(execute_cmd_retry())
            .when(|err| err.kind() == io::ErrorKind::ExecutableFileBusy)
            .notify(|err, duration| {
                debug!("spawn failed with ETXTBSY, retrying in {duration:?}: {err}");
            })
            .call()
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

            let stdout = child.stdout.take().expect("stdout was piped");
            let progress = progress.clone();
            let handle = thread::spawn(move || {
                // Singular mode handles one pack per command instance.
                read_progress_output(stdout, 1, &progress)
            });
            Ok((child, id, handle))
        })
        .collect::<RusticResult<_>>()?;

    let mut failed_ids = Vec::new();

    for (mut child, id, handle) in readers {
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

        let current = handle.join().map_err(|err| {
            let msg = format!("Thread panicked: {err:?}");
            RusticError::new(
                ErrorKind::ExternalCommand,
                format!("Error joining warm-up command thread: {msg}"),
            )
            .attach_context("command", command.to_string())
            .attach_context("id", &id)
            .attach_context("type", format!("{ty:?}"))
        })?;

        if status.success() {
            finalize_progress(current, 1, progress);
        } else {
            failed_ids.push((id, status));
        }
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

    let invocation_size = batch.len() as u64;

    let mut child = (|| {
        Command::new(command.command())
            .args(&args)
            .stdout(Stdio::piped())
            .spawn()
    })
    .retry(execute_cmd_retry())
    .when(|err| err.kind() == io::ErrorKind::ExecutableFileBusy)
    .notify(|err, duration| {
        debug!("spawn failed with ETXTBSY, retrying in {duration:?}: {err}");
    })
    .call()
    .map_err(|err| {
        RusticError::with_source(
            ErrorKind::ExternalCommand,
            "Error in executing warm-up command `{command}`.",
            err,
        )
        .attach_context("command", command.to_string())
        .attach_context("type", format!("{ty:?}"))
    })?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let progress_for_thread = progress.clone();
    let handle =
        thread::spawn(move || read_progress_output(stdout, invocation_size, &progress_for_thread));

    let status = child.wait().map_err(|err| {
        RusticError::with_source(
            ErrorKind::ExternalCommand,
            "Error waiting for warm-up command `{command}`.",
            err,
        )
        .attach_context("command", command.to_string())
        .attach_context("type", format!("{ty:?}"))
    })?;

    let current = handle.join().map_err(|err| {
        let msg = format!("Thread panicked: {err:?}");
        RusticError::new(
            ErrorKind::ExternalCommand,
            format!("Error joining warm-up command thread: {msg}"),
        )
        .attach_context("command", command.to_string())
        .attach_context("batch_size", batch.len().to_string())
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

    finalize_progress(current, invocation_size, progress);

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
