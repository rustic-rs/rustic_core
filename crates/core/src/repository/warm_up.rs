use std::process::Command;
use std::thread::sleep;

use log::{debug, error};
use rayon::ThreadPoolBuilder;

use crate::{
    CommandInput,
    backend::{FileType, ReadBackend},
    error::{ErrorKind, RusticError, RusticResult},
    progress::{Progress, ProgressBars},
    repofile::packfile::PackId,
    repository::{Repository, WarmUpPackIdInput, WarmUpInputType},
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
        warm_up_command(
            packs,
            warm_up_wait_cmd,
            &repo.pb,
            &WarmUpType::WaitPack,
            repo.opts.warm_up_batch,
            repo.opts.warm_up_pack_id_input.unwrap_or_default(),
            repo.opts.warm_up_input_type.unwrap_or_default(),
            &repo.be,
        )?;
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
        warm_up_command(
            packs,
            warm_up_cmd,
            &repo.pb,
            &WarmUpType::WarmUp,
            repo.opts.warm_up_batch,
            repo.opts.warm_up_pack_id_input.unwrap_or_default(),
            repo.opts.warm_up_input_type.unwrap_or_default(),
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

/// Get the input string for a pack ID based on the input type
fn get_pack_input(pack_id: &PackId, input_type: WarmUpInputType, backend: &impl ReadBackend) -> RusticResult<String> {
    match input_type {
        WarmUpInputType::PackId => Ok(pack_id.to_hex().to_string()),
        WarmUpInputType::BackendPath => backend.warmup_path(FileType::Pack, pack_id),
    }
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
/// * `input_mode` - How to pass pack IDs to the command.
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
    input_mode: WarmUpPackIdInput,
    input_type: WarmUpInputType,
    backend: &impl ReadBackend,
) -> RusticResult<()> {
    let packs: Vec<_> = packs.collect();
    let total_packs = packs.len();

    let p = pb.progress_counter(match ty {
        WarmUpType::WarmUp => "warming up packs...",
        WarmUpType::WaitPack => "waiting for packs to be ready...",
    });
    p.set_length(total_packs as u64);

    // Process packs in batches
    for batch in packs.chunks(batch_size) {
        match input_mode {
            WarmUpPackIdInput::Anchor => {
                // For anchor mode, call command once per pack with %id replacement
                for pack in batch {
                    let pack_input = get_pack_input(pack, input_type, backend)
                        .map_err(|err| {
                            RusticError::with_source(
                                ErrorKind::Backend,
                                "Failed to get backend path for pack. This backend may not support path-based warm-up operations.",
                                err,
                            )
                            .attach_context("pack", pack.to_hex().to_string())
                            .attach_context("input_type", format!("{:?}", input_type))
                        })?;
                    let args: Vec<_> = command
                        .args()
                        .iter()
                        .map(|c| c.replace("%id", &pack_input))
                        .collect();

                    debug!("calling {command:?} for pack {pack:?}...");

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
                            format!("{ty:?} command failed for pack {pack:?}. {status}"),
                        )
                        .attach_context("command", command.to_string())
                        .attach_context("pack", pack.to_hex().to_string())
                        .attach_context("status", status.to_string())
                        .attach_context("type", format!("{ty:?}")));
                    }
                    p.inc(1);
                }
            }
            WarmUpPackIdInput::Argv => {
                // For argv mode, pass all pack inputs in batch as arguments
                let mut args = command.args().to_vec();
                for pack in batch {
                    let pack_input = get_pack_input(pack, input_type, backend)
                        .map_err(|err| {
                            RusticError::with_source(
                                ErrorKind::Backend,
                                "Failed to get backend path for pack. This backend may not support path-based warm-up operations.",
                                err,
                            )
                            .attach_context("pack", pack.to_hex().to_string())
                            .attach_context("input_type", format!("{:?}", input_type))
                        })?;
                    args.push(pack_input);
                }

                debug!(
                    "calling {command:?} with {} pack(s)...",
                    batch.len()
                );

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
                        format!("{ty:?} command failed for batch of {} pack(s). {status}", batch.len()),
                    )
                    .attach_context("command", command.to_string())
                    .attach_context("batch_size", batch.len().to_string())
                    .attach_context("status", status.to_string())
                    .attach_context("type", format!("{ty:?}")));
                }
                p.inc(batch.len() as u64);
            }
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
