use bytes::Bytes;
use typed_path::UnixPath;

use crate::{
    backend::{FileType, FindInBackend, decrypt::DecryptReadBackend},
    blob::{BlobId, BlobType, tree::Tree},
    error::{ErrorKind, RusticError, RusticResult},
    index::ReadIndex,
    progress::ProgressBars,
    repofile::SnapshotFile,
    repository::{IndexedFull, IndexedTree, Open, Repository},
};

/// Prints the contents of a file.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to read from.
/// * `tpe` - The type of the file.
/// * `id` - The id of the file.
///
/// # Errors
///
/// * If the string is not a valid hexadecimal string
/// * If no id could be found.
/// * If the id is not unique.
///
/// # Returns
///
/// The data read.
pub(crate) fn cat_file<P, S: Open>(
    repo: &Repository<P, S>,
    tpe: FileType,
    id: &str,
) -> RusticResult<Bytes> {
    let id = repo.dbe().find_id(tpe, id)?;
    let data = repo.dbe().read_encrypted_full(tpe, &id)?;
    Ok(data)
}

// TODO: Add documentation!
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to read from.
/// * `tpe` - The type of the file.
/// * `id` - The id of the file.
///
/// # Errors
///
/// * If the string is not a valid hexadecimal string
pub(crate) fn cat_blob<P, S: IndexedFull>(
    repo: &Repository<P, S>,
    tpe: BlobType,
    id: &str,
) -> RusticResult<Bytes> {
    let id = id.parse()?;
    let data = repo.index().blob_from_backend(repo.dbe(), tpe, &id)?;

    Ok(data)
}

/// Prints the contents of a tree.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to read from.
/// * `snap` - The snapshot to read from.
/// * `sn_filter` - The filter to apply to the snapshot.
///
/// # Errors
///
/// * If the path is not a directory.
///
/// # Returns
///
/// The data read.
pub(crate) fn cat_tree<P: ProgressBars, S: IndexedTree>(
    repo: &Repository<P, S>,
    snap: &str,
    sn_filter: impl FnMut(&SnapshotFile) -> bool + Send + Sync,
) -> RusticResult<Bytes> {
    let (id, path) = snap.split_once(':').unwrap_or((snap, ""));
    let snap = SnapshotFile::from_str(
        repo.dbe(),
        id,
        sn_filter,
        &repo.pb.progress_counter("getting snapshot..."),
    )?;
    let node = Tree::node_from_path(repo.dbe(), repo.index(), snap.tree, UnixPath::new(path))?;
    let id = node.subtree.ok_or_else(|| {
        RusticError::new(
            ErrorKind::InvalidInput,
            "Path `{path}` in Node subtree is not a directory. Please provide a directory path.",
        )
        .attach_context("path", path.to_string())
    })?;
    let data = repo
        .index()
        .blob_from_backend(repo.dbe(), BlobType::Tree, &BlobId::from(*id))?;
    Ok(data)
}
