use std::process::Command;
use std::thread::sleep;

use log::{debug, error, warn};
use rayon::ThreadPoolBuilder;

use crate::{
    CommandInput,
    backend::{FileType, ReadBackend},
    error::{ErrorKind, RusticError, RusticResult},
    progress::{Progress, ProgressBars},
    repofile::packfile::PackId,
    repository::{PlaceholderMode, Repository},
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
        let flags = super::detect_placeholders(warm_up_wait_cmd);
        let mode = super::validate_placeholders(flags, warm_up_wait_cmd)?;

        warm_up_command(
            packs,
            warm_up_wait_cmd,
            &repo.pb,
            &WarmUpType::WaitPack,
            repo.opts.warm_up_batch,
            mode,
            &repo.be,
        )?;
    } else if let Some(wait) = repo.opts.warm_up_wait {
        let p = repo.pb.progress_spinner(format!("waiting {wait}..."));
        sleep(
            wait.try_into()
                // ignore conversation errors, but print out warning
                .inspect_err(|err| warn!("cannot wait for warm-up: {err}"))
                .unwrap(),
        );
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
        let flags = super::detect_placeholders(warm_up_cmd);
        let mode = super::validate_placeholders(flags, warm_up_cmd)?;

        warm_up_command(
            packs,
            warm_up_cmd,
            &repo.pb,
            &WarmUpType::WarmUp,
            repo.opts.warm_up_batch,
            mode,
            &repo.be,
        )?;
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

/// Get the ID string for a pack
fn get_pack_id(id: &PackId) -> String {
    id.to_hex().to_string()
}

/// Get the backend path string for a pack
fn get_pack_path(id: &PackId, backend: &impl ReadBackend) -> RusticResult<String> {
    backend.warmup_path(FileType::Pack, id)
}

/// Warm up the repository using a command.
///
/// # Arguments
///
/// * `packs` - The packs to warm up.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `batch_size` - The number of packs to process in each batch.
/// * `mode` - The placeholder mode for how to pass pack data to the command.
/// * `backend` - The backend to get pack paths from.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_command<P: ProgressBars>(
    packs: impl ExactSizeIterator<Item = PackId>,
    command: &CommandInput,
    pb: &P,
    ty: &WarmUpType,
    batch_size: usize,
    mode: PlaceholderMode,
    backend: &impl ReadBackend,
) -> RusticResult<()> {
    let packs: Vec<_> = packs.collect();
    let total_packs = packs.len();

    let p = pb.progress_counter(match ty {
        WarmUpType::WarmUp => "warming up packs...",
        WarmUpType::WaitPack => "waiting for packs to be ready...",
    });
    p.set_length(total_packs as u64);

    for batch in packs.chunks(batch_size) {
        warm_up_batch(batch, command, ty, mode, backend, &p)?;
    }

    p.finish();
    Ok(())
}

/// Warm up a single batch of packs using a command.
///
/// # Arguments
///
/// * `batch` - The packs in this batch.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `mode` - The placeholder mode for how to pass pack data to the command.
/// * `backend` - The backend to get pack paths from.
/// * `progress` - The progress bar to update.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_batch(
    batch: &[PackId],
    command: &CommandInput,
    ty: &WarmUpType,
    mode: PlaceholderMode,
    backend: &impl ReadBackend,
    progress: &impl Progress,
) -> RusticResult<()> {
    match mode {
        PlaceholderMode::Single { use_ids, use_paths } => {
            warm_up_batch_single(batch, command, ty, use_ids, use_paths, backend, progress)
        }
        PlaceholderMode::Multiple { use_ids, use_paths } => {
            warm_up_batch_multiple(batch, command, ty, use_ids, use_paths, backend, progress)
        }
    }
}

/// Warm up a batch of packs using single mode (one command per pack).
///
/// # Arguments
///
/// * `batch` - The packs in this batch.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `use_ids` - Whether to use pack IDs.
/// * `use_paths` - Whether to use pack paths.
/// * `backend` - The backend to get pack paths from.
/// * `progress` - The progress bar to update.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_batch_single(
    batch: &[PackId],
    command: &CommandInput,
    ty: &WarmUpType,
    use_ids: bool,
    use_paths: bool,
    backend: &impl ReadBackend,
    progress: &impl Progress,
) -> RusticResult<()> {
    let mut children = Vec::new();
    let mut pack_ids_for_error = Vec::new();

    for pack in batch {
        let id_value = if use_ids {
            Some(get_pack_id(pack))
        } else {
            None
        };

        let path_value = if use_paths {
            Some(get_pack_path(pack, backend).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Failed to get backend path for pack. This backend may not support path-based warm-up operations.",
                    err,
                )
                .attach_context("pack", pack.to_hex().to_string())
            })?)
        } else {
            None
        };

        let args: Vec<_> = command
            .args()
            .iter()
            .map(|c| {
                let mut arg = c.clone();
                if let Some(ref id) = id_value {
                    arg = arg.replace("%id", id);
                }
                if let Some(ref path) = path_value {
                    arg = arg.replace("%path", path);
                }
                arg
            })
            .collect();

        debug!("spawning {command:?} for pack {pack:?}...");

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
                .attach_context("pack", pack.to_hex().to_string())
                .attach_context("type", format!("{ty:?}"))
            })?;

        children.push(child);
        pack_ids_for_error.push(pack);
    }

    let mut failed_packs = Vec::new();

    for (i, mut child) in children.into_iter().enumerate() {
        let pack = pack_ids_for_error[i];

        debug!("waiting for warm-up command for pack {pack:?}...");

        let status = child.wait().map_err(|err| {
            RusticError::with_source(
                ErrorKind::ExternalCommand,
                "Error waiting for warm-up command `{command}`.",
                err,
            )
            .attach_context("command", command.to_string())
            .attach_context("pack", pack.to_hex().to_string())
            .attach_context("type", format!("{ty:?}"))
        })?;

        if !status.success() {
            failed_packs.push((pack, status));
        }

        progress.inc(1);
    }

    if !failed_packs.is_empty() {
        let error_msg = if failed_packs.len() == 1 {
            let (pack, status) = &failed_packs[0];
            format!("{ty:?} command failed for pack {pack:?}. {status}")
        } else {
            format!(
                "{ty:?} command failed for {}/{} pack(s): {}",
                failed_packs.len(),
                batch.len(),
                failed_packs
                    .iter()
                    .map(|(pack, status)| format!("{pack:?} ({status})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        return Err(RusticError::new(ErrorKind::ExternalCommand, error_msg)
            .attach_context("command", command.to_string())
            .attach_context("failed_packs", failed_packs.len().to_string())
            .attach_context("total_packs", batch.len().to_string())
            .attach_context("type", format!("{ty:?}")));
    }

    Ok(())
}

/// Warm up a batch of packs using multiple mode (single command with all values).
///
/// # Arguments
///
/// * `batch` - The packs in this batch.
/// * `command` - The command to execute.
/// * `pb` - The progress bar to use.
/// * `ty` - The type of warm-up operation.
/// * `use_ids` - Whether to use pack IDs.
/// * `use_paths` - Whether to use pack paths.
/// * `backend` - The backend to get pack paths from.
/// * `progress` - The progress bar to update.
///
/// # Errors
///
/// * If the command could not be parsed.
fn warm_up_batch_multiple(
    batch: &[PackId],
    command: &CommandInput,
    ty: &WarmUpType,
    use_ids: bool,
    use_paths: bool,
    backend: &impl ReadBackend,
    progress: &impl Progress,
) -> RusticResult<()> {
    let mut args = Vec::new();

    let mut id_values = Vec::new();
    let mut path_values = Vec::new();

    for pack in batch {
        if use_ids {
            id_values.push(get_pack_id(pack));
        }
        if use_paths {
            path_values.push(get_pack_path(pack, backend).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Failed to get backend path for pack. This backend may not support path-based warm-up operations.",
                    err,
                )
                .attach_context("pack", pack.to_hex().to_string())
            })?);
        }
    }

    for arg in command.args() {
        if use_ids && arg.contains("%ids") {
            args.extend(id_values.clone());
        } else if use_paths && arg.contains("%paths") {
            args.extend(path_values.clone());
        } else {
            args.push(arg.clone());
        }
    }

    debug!("calling {command:?} with {} pack(s)...", batch.len());

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
                "{ty:?} command failed for batch of {} pack(s). {status}",
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
                }
                progress_bar_ref.inc(1);
            });
        }
    });

    progress_bar_ref.finish();

    Ok(())
}
