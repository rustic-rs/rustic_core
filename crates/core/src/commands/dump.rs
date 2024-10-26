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
/// * [`CommandErrorKind::DumpNotSupported`] - If the node is not a file.
///
/// [`CommandErrorKind::DumpNotSupported`]: crate::error::CommandErrorKind::DumpNotSupported
pub(crate) fn dump<P, S: IndexedFull>(
    repo: &Repository<P, S>,
    node: &Node,
    w: &mut impl Write,
) -> RusticResult<()> {
    if node.node_type != NodeType::File {
        return Err(RusticError::new(
            ErrorKind::Command,
            "Dump is not supported for non-file node types. You could try to use `cat` instead.",
        )
        .attach_context("node type", node.node_type.to_string()));
    }

    for id in node.content.as_ref().unwrap() {
        let data = repo.get_blob_cached(&BlobId::from(**id), BlobType::Data)?;
        w.write_all(&data).map_err(|err| {
            RusticError::with_source(ErrorKind::Io, "Failed to write data to writer.", err)
        })?;
    }
    Ok(())
}
