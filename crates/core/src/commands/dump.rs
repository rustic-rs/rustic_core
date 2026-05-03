use std::{io::Write, num::NonZeroUsize, thread::scope};

use derive_setters::Setters;
use pariter::IteratorExt;

use crate::{
    backend::node::{Node, NodeType},
    blob::{BlobId, BlobType, DataId},
    error::{ErrorKind, RusticError, RusticResult},
    repository::{IndexedFull, Repository},
};

pub(crate) mod constants {
    /// Minimum blob count required to enable parallel fetching.
    ///
    /// For files that decompose into a single blob there is nothing to overlap,
    /// so we stay on the sequential path to avoid the worker-thread setup cost.
    pub(crate) const PARALLEL_DUMP_MIN_BLOBS: usize = 2;
}

/// Options for the `dump` command.
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Debug, Copy, Clone, Default, Setters)]
#[setters(into)]
#[non_exhaustive]
pub struct DumpOptions {
    /// Number of reader threads used to fetch blobs in parallel.
    ///
    /// `0` selects the available parallelism reported by the runtime.
    /// `1` forces the sequential implementation.
    #[cfg_attr(feature = "clap", clap(long, default_value = "0"))]
    pub num_threads: u32,
}

impl DumpOptions {
    /// Resolve the configured thread count to a concrete value.
    ///
    /// Returns `None` for the sequential path and `Some(n)` for `n` worker
    /// threads.
    fn resolved_threads(self, blob_count: usize) -> Option<NonZeroUsize> {
        if blob_count < constants::PARALLEL_DUMP_MIN_BLOBS {
            return None;
        }
        let threads = match self.num_threads {
            0 => std::thread::available_parallelism().map_or(1, NonZeroUsize::get),
            n => n as usize,
        };
        NonZeroUsize::new(threads).filter(|n| n.get() > 1)
    }
}

/// Dumps the contents of a file.
///
/// # Type Parameters
///
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to read from.
/// * `node` - The node to dump.
/// * `w` - The writer to write to.
/// * `opts` - The dump options to use.
///
/// # Errors
///
/// * If the node is not a file.
/// * If a blob cannot be fetched from the backend.
/// * If writing to `w` fails.
pub(crate) fn dump<S: IndexedFull + Sync>(
    repo: &Repository<S>,
    node: &Node,
    w: &mut impl Write,
    opts: DumpOptions,
) -> RusticResult<()> {
    if node.node_type != NodeType::File {
        return Err(RusticError::new(
            ErrorKind::Unsupported,
            "Dump is not supported for non-file node types `{node_type}`. You could try to use `cat` instead.",
        )
        .attach_context("node_type", node.node_type.to_string()));
    }

    let Some(content) = node.content.as_ref() else {
        return Ok(());
    };

    match opts.resolved_threads(content.len()) {
        None => dump_sequential(repo, content, w),
        Some(threads) => dump_parallel(repo, content, w, threads),
    }
}

fn dump_sequential<S: IndexedFull>(
    repo: &Repository<S>,
    content: &[DataId],
    w: &mut impl Write,
) -> RusticResult<()> {
    for id in content {
        let data = repo.get_blob_cached(&BlobId::from(**id), BlobType::Data)?;
        write_blob(w, &data)?;
    }
    Ok(())
}

fn dump_parallel<S: IndexedFull + Sync>(
    repo: &Repository<S>,
    content: &[DataId],
    w: &mut impl Write,
    threads: NonZeroUsize,
) -> RusticResult<()> {
    let threads = threads.get();

    scope(|s| -> RusticResult<()> {
        content
            .iter()
            .map(|id| BlobId::from(**id))
            .parallel_map_scoped_custom(
                s,
                |b| b.threads(threads).buffer_size(threads * 2),
                |id| repo.get_blob_cached(&id, BlobType::Data),
            )
            .try_for_each(|res| write_blob(w, &res?))
    })
}

fn write_blob(w: &mut impl Write, data: &[u8]) -> RusticResult<()> {
    w.write_all(data).map_err(|err| {
        RusticError::with_source(
            ErrorKind::InputOutput,
            "Failed to write data to writer.",
            err,
        )
    })
}
