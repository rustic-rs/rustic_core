use std::io::Write;

use crate::{
    backend::node::{Node, NodeType},
    blob::BlobType,
    error::{CommandErrorKind, RusticResult},
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
        return Err(CommandErrorKind::DumpNotSupported(node.node_type.clone()).into());
    }

    for id in node.content.iter().flatten() {
        let data = repo.get_blob_cached(id, BlobType::Data)?;
        w.write_all(&data)?;
    }
    Ok(())
}
