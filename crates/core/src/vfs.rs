mod format;

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use bytes::{Bytes, BytesMut};
use runtime_format::FormatArgs;
use strum::EnumString;

use crate::{
    blob::{BlobId, DataId, tree::TreeId},
    error::{ErrorKind, RusticError, RusticResult},
    index::ReadIndex,
    repofile::{BlobType, Metadata, Node, NodeType, SnapshotFile},
    repository::{IndexedFull, Repository},
    vfs::format::FormattedSnapshot,
};

/// [`VfsErrorKind`] describes the errors that can be returned from the Virtual File System
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum VfsErrorKind {
    /// Directory exists as non-virtual directory
    DirectoryExistsAsNonVirtual,
    /// Only normal paths allowed
    OnlyNormalPathsAreAllowed,
    /// Name `{0:?}` doesn't exist
    NameDoesNotExist(OsString),
}

pub(crate) type VfsResult<T> = Result<T, VfsErrorKind>;

#[derive(Debug, Clone, Copy)]
/// `IdenticalSnapshot` describes how to handle identical snapshots.
pub enum IdenticalSnapshot {
    /// create a link to the previous identical snapshots
    AsLink,
    /// make a dir, i.e. don't add special treatment for identical snapshots
    AsDir,
}

#[derive(Debug, Clone, Copy)]
/// `Latest` describes whether a `latest` entry should be added.
pub enum Latest {
    /// Add `latest` as directory with identical content as the last snapshot by time.
    AsDir,
    /// Add `latest` as symlink
    AsLink,
    /// Don't add a `latest` entry
    No,
}

#[derive(Debug)]
/// A potentially virtual tree in the [`Vfs`]
enum VfsTree {
    /// A symlink to the given link target
    Link(OsString),
    /// A repository tree; id of the tree
    RusticTree(TreeId),
    /// A purely virtual tree containing subtrees
    VirtualTree(BTreeMap<OsString, Self>),
}

#[derive(Debug)]
/// A resolved path within a [`Vfs`]
enum VfsPath<'a> {
    /// Path is the given symlink
    Link(&'a OsString),
    /// Path is within repository, give the tree [`Id`] and remaining path.
    RusticPath(&'a TreeId, PathBuf),
    /// Path is the given virtual tree
    VirtualTree(&'a BTreeMap<OsString, VfsTree>),
}

impl VfsTree {
    /// Create a new [`VfsTree`]
    fn new() -> Self {
        Self::VirtualTree(BTreeMap::new())
    }

    /// Add some tree to this root tree at the given path
    ///
    /// # Arguments
    ///
    /// * `path` - The path to add the tree to
    /// * `new_tree` - The tree to add
    ///
    /// # Errors
    ///
    /// * If the path is not a normal path
    /// * If the path is a directory in the repository
    ///
    /// # Returns
    ///
    /// `Ok(())` if the tree was added successfully
    fn add_tree(&mut self, path: &Path, new_tree: Self) -> VfsResult<()> {
        let mut tree = self;
        let mut components = path.components();
        let Some(Component::Normal(last)) = components.next_back() else {
            return Err(VfsErrorKind::OnlyNormalPathsAreAllowed);
        };

        for comp in components {
            if let Component::Normal(name) = comp {
                match tree {
                    Self::VirtualTree(virtual_tree) => {
                        tree = virtual_tree
                            .entry(name.to_os_string())
                            .or_insert(Self::VirtualTree(BTreeMap::new()));
                    }
                    _ => {
                        return Err(VfsErrorKind::DirectoryExistsAsNonVirtual);
                    }
                }
            }
        }

        let Self::VirtualTree(virtual_tree) = tree else {
            return Err(VfsErrorKind::DirectoryExistsAsNonVirtual);
        };

        _ = virtual_tree.insert(last.to_os_string(), new_tree);
        Ok(())
    }

    /// Get the tree at this given path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to get the tree for
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// If the path is within a real repository tree, this returns the [`VfsTree::RusticTree`] and the remaining path
    fn get_path(&self, path: &Path) -> VfsResult<VfsPath<'_>> {
        let mut tree = self;
        let mut components = path.components();
        loop {
            match tree {
                Self::RusticTree(id) => {
                    let path: PathBuf = components.collect();
                    return Ok(VfsPath::RusticPath(id, path));
                }
                Self::VirtualTree(virtual_tree) => match components.next() {
                    Some(Component::Normal(name)) => {
                        if let Some(new_tree) = virtual_tree.get(name) {
                            tree = new_tree;
                        } else {
                            return Err(VfsErrorKind::NameDoesNotExist(name.to_os_string()));
                        }
                    }
                    None => {
                        return Ok(VfsPath::VirtualTree(virtual_tree));
                    }

                    _ => {}
                },
                Self::Link(target) => return Ok(VfsPath::Link(target)),
            }
        }
    }
}

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, EnumString, serde::Deserialize, serde::Serialize)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
/// Policy to describe how to handle access to a file
pub enum FilePolicy {
    /// Don't allow reading the file
    Forbidden,
    /// Read the file
    Read,
}

#[derive(Debug)]
/// A virtual file system which offers repository contents
pub struct Vfs {
    /// The root tree
    tree: VfsTree,
}

impl Vfs {
    /// Create a new [`Vfs`] from a directory [`Node`].
    ///
    /// # Arguments
    ///
    /// * `node` - The directory [`Node`] to create the [`Vfs`] from
    ///
    /// # Panics
    ///
    /// * If the node is not a directory
    #[must_use]
    pub fn from_dir_node(node: &Node) -> Self {
        let tree = VfsTree::RusticTree(node.subtree.unwrap());
        Self { tree }
    }

    /// Create a new [`Vfs`] from a list of snapshots.
    ///
    /// # Arguments
    ///
    /// * `snapshots` - The snapshots to create the [`Vfs`] from
    /// * `path_template` - The template for the path of the snapshots
    /// * `time_template` - The template for the time of the snapshots
    /// * `latest_option` - Whether to add a `latest` entry
    /// * `id_snap_option` - Whether to add a link to identical snapshots
    ///
    /// # Errors
    ///
    /// * If the path is not a normal path
    /// * If the path is a directory in the repository
    #[allow(clippy::too_many_lines)]
    pub fn from_snapshots(
        mut snapshots: Vec<SnapshotFile>,
        path_template: &str,
        time_template: &str,
        latest_option: Latest,
        id_snap_option: IdenticalSnapshot,
    ) -> RusticResult<Self> {
        snapshots.sort_unstable();
        let mut tree = VfsTree::new();

        // to handle identical trees
        let mut last_parent = None;
        let mut last_name = None;
        let mut last_tree = TreeId::default();

        // to handle "latest" entries
        let mut dirs_for_link = BTreeMap::new();
        let mut dirs_for_snap = BTreeMap::new();

        for snap in snapshots {
            let path = FormatArgs::new(
                path_template,
                &FormattedSnapshot {
                    snap: &snap,
                    time_format: time_template,
                },
            )
            .to_string();
            let path = Path::new(&path);
            let filename = path.file_name().map(OsStr::to_os_string);
            let parent_path = path.parent().map(Path::to_path_buf);

            // Save paths for latest entries, if requested
            if matches!(latest_option, Latest::AsLink) {
                _ = dirs_for_link.insert(parent_path.clone(), filename.clone());
            }
            if matches!(latest_option, Latest::AsDir) {
                _ = dirs_for_snap.insert(parent_path.clone(), snap.tree);
            }

            // Create the entry, potentially as symlink if requested
            if last_parent != parent_path || last_name != filename {
                if matches!(id_snap_option, IdenticalSnapshot::AsLink)
                    && last_parent == parent_path
                    && last_tree == snap.tree
                {
                    if let Some(name) = last_name {
                        tree.add_tree(path, VfsTree::Link(name.clone()))
                            .map_err(|err| {
                                RusticError::with_source(
                                    ErrorKind::Vfs,
                                    "Failed to add a link `{name}` to root tree at `{path}`",
                                    err,
                                )
                                .attach_context("path", path.display().to_string())
                                .attach_context("name", name.to_string_lossy())
                                .ask_report()
                            })?;
                    }
                } else {
                    tree.add_tree(path, VfsTree::RusticTree(snap.tree))
                        .map_err(|err| {
                            RusticError::with_source(
                                ErrorKind::Vfs,
                                "Failed to add repository tree `{tree_id}` to root tree at `{path}`",
                                err,
                            )
                            .attach_context("path", path.display().to_string())
                            .attach_context("tree_id", snap.tree.to_string())
                            .ask_report()
                        })?;
                }
            }
            last_parent = parent_path;
            last_name = filename;
            last_tree = snap.tree;
        }

        // Add latest entries if requested
        match latest_option {
            Latest::No => {}
            Latest::AsLink => {
                for (path, target) in dirs_for_link {
                    if let (Some(mut path), Some(target)) = (path, target) {
                        path.push("latest");
                        tree.add_tree(&path, VfsTree::Link(target.clone()))
                            .map_err(|err| {
                                RusticError::with_source(
                                    ErrorKind::Vfs,
                                    "Failed to link latest `{target}` entry to root tree at `{path}`",
                                    err,
                                )
                                .attach_context("path", path.display().to_string())
                                .attach_context("target", target.to_string_lossy())
                                .attach_context("latest", "link")
                                .ask_report()
                            })?;
                    }
                }
            }
            Latest::AsDir => {
                for (path, subtree) in dirs_for_snap {
                    if let Some(mut path) = path {
                        path.push("latest");
                        tree.add_tree(&path, VfsTree::RusticTree(subtree))
                            .map_err(|err| {
                                RusticError::with_source(
                                    ErrorKind::Vfs,
                                    "Failed to add latest subtree id `{id}` to root tree at `{path}`",
                                    err,
                                )
                                .attach_context("path", path.display().to_string())
                                .attach_context("tree_id", subtree.to_string())
                                .attach_context("latest", "dir")
                                .ask_report()
                            })?;
                    }
                }
            }
        }
        Ok(Self { tree })
    }

    /// Get a [`Node`] from the specified path.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to get the [`Node`] from
    /// * `path` - The path to get the [`Tree`] at
    ///
    /// # Errors
    ///
    /// * If the component name doesn't exist
    ///
    /// # Returns
    ///
    /// The [`Node`] at the specified path
    ///
    /// [`Tree`]: crate::repofile::Tree
    pub fn node_from_path<S: IndexedFull>(
        &self,
        repo: &Repository<S>,
        path: &Path,
    ) -> RusticResult<Node> {
        let meta = Metadata::default();
        match self.tree.get_path(path).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Vfs,
                "Failed to get tree at given path `{path}`",
                err,
            )
            .attach_context("path", path.display().to_string())
            .ask_report()
        })? {
            VfsPath::RusticPath(tree_id, path) => Ok(repo.node_from_path(*tree_id, &path)?),
            VfsPath::VirtualTree(_) => {
                Ok(Node::new(String::new(), NodeType::Dir, meta, None, None))
            }
            VfsPath::Link(target) => Ok(Node::new(
                String::new(),
                NodeType::from_link(Path::new(target)),
                meta,
                None,
                None,
            )),
        }
    }

    /// Get a list of [`Node`]s from the specified directory path.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to get the [`Node`] from
    /// * `path` - The path to get the [`Tree`] at
    ///
    /// # Errors
    ///
    /// * If the component name doesn't exist
    ///
    /// # Returns
    ///
    /// The list of [`Node`]s at the specified path
    ///
    /// [`Tree`]: crate::repofile::Tree
    ///
    /// # Panics
    ///
    /// * Panics if the path is not a directory.
    pub fn dir_entries_from_path<S: IndexedFull>(
        &self,
        repo: &Repository<S>,
        path: &Path,
    ) -> RusticResult<Vec<Node>> {
        let result = match self.tree.get_path(path).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Vfs,
                "Failed to get tree at given path `{path}`",
                err,
            )
            .attach_context("path", path.display().to_string())
            .ask_report()
        })? {
            VfsPath::RusticPath(tree_id, path) => {
                let node = repo.node_from_path(*tree_id, &path)?;
                if node.is_dir() {
                    let tree = repo.get_tree(&node.subtree.unwrap())?;
                    tree.nodes
                } else {
                    Vec::new()
                }
            }
            VfsPath::VirtualTree(virtual_tree) => virtual_tree
                .iter()
                .map(|(name, tree)| {
                    let node_type = match tree {
                        VfsTree::Link(target) => NodeType::from_link(Path::new(target)),
                        _ => NodeType::Dir,
                    };
                    Node::new_node(name, node_type, Metadata::default())
                })
                .collect(),
            VfsPath::Link(str) => {
                return Err(RusticError::new(
                    ErrorKind::Vfs,
                    "No directory entries for symlink `{symlink}` found. Is the path valid unicode?",
                )
                .attach_context("symlink", str.to_string_lossy().to_string()));
            }
        };
        Ok(result)
    }
}

/// `OpenFile` stores all information needed to access the contents of a file node
#[derive(Debug)]
pub struct OpenFile {
    // The list of blobs
    content: Vec<DataId>,
    startpoints: ContentStartpoints,
}

impl OpenFile {
    /// Create an `OpenFile` from a file `Node`
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to create the `OpenFile` for
    /// * `node` - The `Node` to create the `OpenFile` for
    ///
    /// # Errors
    /// - If the index for the needed data blobs cannot be read
    ///
    /// # Returns
    ///
    /// The created `OpenFile`
    pub(crate) fn from_node<S: IndexedFull>(
        repo: &Repository<S>,
        node: &Node,
    ) -> RusticResult<Self> {
        let content: Vec<_> = node.content.clone().unwrap_or_default();

        let startpoints = ContentStartpoints::from_sizes(content.iter().map(|id| {
            Ok(repo
                .index()
                .get_data(id)
                .ok_or_else(|| {
                    RusticError::new(ErrorKind::Vfs, "blob {blob} is not contained in index")
                        .attach_context("blob", id.to_string())
                })?
                .data_length() as usize)
        }))?;

        Ok(Self {
            content,
            startpoints,
        })
    }

    /// Read the `OpenFile` at the given `offset` from the `repo`.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to read the `OpenFile` from
    /// * `offset` - The offset to read the `OpenFile` from
    /// * `length` - The length of the content to read from the `OpenFile`
    ///
    /// # Errors
    ///
    /// - if reading the needed blob(s) from the backend fails
    ///
    /// # Returns
    ///
    /// The read bytes from the given offset and length.
    /// If offset is behind the end of the file, an empty `Bytes` is returned.
    /// If length is too large, the result up to the end of the file is returned.
    pub fn read_at<S: IndexedFull>(
        &self,
        repo: &Repository<S>,
        offset: usize,
        mut length: usize,
    ) -> RusticResult<Bytes> {
        let (mut i, mut offset) = self.startpoints.compute_start(offset);

        let mut result = BytesMut::with_capacity(length);

        // The case of empty node.content is also correctly handled here
        while length > 0 && i < self.content.len() {
            let data = repo.get_blob_cached(&BlobId::from(self.content[i]), BlobType::Data)?;

            if offset > data.len() {
                // we cannot read behind the blob. This only happens if offset is too large to fit in the last blob
                break;
            }

            let to_copy = (data.len() - offset).min(length);
            result.extend_from_slice(&data[offset..offset + to_copy]);
            offset = 0;
            length -= to_copy;
            i += 1;
        }

        Ok(result.into())
    }
}

// helper struct holding blob startpoints of the content
#[derive(Debug)]
struct ContentStartpoints(Vec<usize>);

impl ContentStartpoints {
    fn from_sizes(sizes: impl IntoIterator<Item = RusticResult<usize>>) -> RusticResult<Self> {
        let mut start = 0;
        let mut offsets: Vec<_> = sizes
            .into_iter()
            .map(|size| -> RusticResult<_> {
                let starts_at = start;
                start += size?;
                Ok(starts_at)
            })
            .collect::<RusticResult<_>>()?;

        if !offsets.is_empty() {
            // offsets is assumed to be partitioned, so we add a starts_at:MAX entry
            offsets.push(usize::MAX);
        }
        Ok(Self(offsets))
    }

    // compute the correct blobid and effective offset from a file offset
    fn compute_start(&self, mut offset: usize) -> (usize, usize) {
        if self.0.is_empty() {
            return (0, 0);
        }
        // find the start of relevant blobs => find the largest index such that self.offsets[i] <= offset, but
        // self.offsets[i+1] > offset  (note that a last dummy element with usize::MAX has been added to ensure we always have two partitions)
        // If offsets is non-empty, then offsets[0] = 0, hence partition_point returns an index >=1.
        let i = self.0.partition_point(|o| o <= &offset) - 1;
        offset -= self.0[i];
        (i, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // helper func
    fn startpoints_from_ok_sizes(sizes: impl IntoIterator<Item = usize>) -> ContentStartpoints {
        ContentStartpoints::from_sizes(sizes.into_iter().map(Ok)).unwrap()
    }

    #[test]
    fn content_offsets_empty_sizes() {
        let offsets = startpoints_from_ok_sizes([]);
        assert_eq!(offsets.compute_start(0), (0, 0));
        assert_eq!(offsets.compute_start(42), (0, 0));
    }

    #[test]
    fn content_offsets_size() {
        let offsets = startpoints_from_ok_sizes([15]);
        assert_eq!(offsets.compute_start(0), (0, 0));
        assert_eq!(offsets.compute_start(5), (0, 5));
        assert_eq!(offsets.compute_start(20), (0, 20));
    }
    #[test]
    fn content_offsets_sizes() {
        let offsets = startpoints_from_ok_sizes([15, 24]);
        assert_eq!(offsets.compute_start(0), (0, 0));
        assert_eq!(offsets.compute_start(5), (0, 5));
        assert_eq!(offsets.compute_start(20), (1, 5));
        assert_eq!(offsets.compute_start(42), (1, 27));
    }
}
