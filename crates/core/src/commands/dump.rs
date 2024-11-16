use std::io::Write;

use crate::{
    backend::node::{Node, NodeType},
    blob::{BlobId, BlobType},
    error::{ErrorKind, RusticError, RusticResult},
    repository::{IndexedFull, Repository},
};

/// Dumps the contents of a file.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
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
pub(crate) fn dump<P, S: IndexedFull>(
    repo: &Repository<P, S>,
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

    for id in node.content.as_ref().unwrap() {
        let data = repo.get_blob_cached(&BlobId::from(**id), BlobType::Data)?;
        w.write_all(&data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to write data to writer.",
                err,
            )
        })?;
    }
    Ok(())
}
