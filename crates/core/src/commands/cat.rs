use std::path::Path;

use bytes::Bytes;

use crate::{
    backend::{decrypt::DecryptReadBackend, FileType, FindInBackend},
    blob::{tree::Tree, BlobId, BlobType},
    error::{CommandErrorKind, RusticResult},
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
/// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
/// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
/// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
///
/// # Returns
///
/// The data read.
///
/// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
/// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
/// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
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
/// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
///
/// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
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
/// * [`CommandErrorKind::PathIsNoDir`] - If the path is not a directory.
///
/// # Returns
///
/// The data read.
///
/// [`CommandErrorKind::PathIsNoDir`]: crate::error::CommandErrorKind::PathIsNoDir
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
    let node = Tree::node_from_path(repo.dbe(), repo.index(), snap.tree, Path::new(path))?;
    let id = node
        .subtree
        .ok_or_else(|| CommandErrorKind::PathIsNoDir(path.to_string()))?;
    let data = repo
        .index()
        .blob_from_backend(repo.dbe(), BlobType::Tree, &BlobId::from(*id))?;
    Ok(data)
}
