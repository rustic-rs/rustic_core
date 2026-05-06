use std::{io::Write, thread::scope};

use pariter::IteratorExt;

use crate::{
    backend::node::{Node, NodeType},
    blob::{BlobId, BlobType, DataId},
    error::{ErrorKind, RusticError, RusticResult},
    repository::{IndexedFull, Repository},
};

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

    // Single-blob files have nothing to overlap, so skip the worker setup.
    if content.len() < 2 {
        return dump_sequential(repo, content, w);
    }

    scope(|s| -> RusticResult<()> {
        content
            .iter()
            .map(|id| BlobId::from(**id))
            .parallel_map_scoped(s, |id| repo.get_blob_cached(&id, BlobType::Data))
            .try_for_each(|res| write_blob(w, &res?))
    })
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

fn write_blob(w: &mut impl Write, data: &[u8]) -> RusticResult<()> {
    w.write_all(data).map_err(|err| {
        RusticError::with_source(
            ErrorKind::InputOutput,
            "Failed to write data to writer.",
            err,
        )
    })
}
