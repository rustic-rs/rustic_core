mod format;
#[cfg(feature = "webdav")]
mod webdavfs;

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use bytes::{Bytes, BytesMut};
use runtime_format::FormatArgs;
use strum::EnumString;

#[cfg(feature = "webdav")]
/// A struct which enables `WebDAV` access to a [`Vfs`] using [`dav-server`]
pub use webdavfs::WebDavFS;

use crate::{
    error::VfsErrorKind,
    repofile::{BlobType, Metadata, Node, NodeType, SnapshotFile},
};
use crate::{
    index::ReadIndex,
    repository::{IndexedFull, IndexedTree, Repository},
    Id, RusticResult,
};

use format::FormattedSnapshot;

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
    /// Don't add a `latest` entry
    No,
    /// Add `latest` as symlink
    AsLink,
    /// Add `latest` as directory with identical content as the last snapshot by time.
    AsDir,
}

#[derive(Debug)]
/// A potentially virtual tree in the [`Vfs`]
enum VfsTree {
    /// A repository tree; id of the tree
    RusticTree(Id),
    /// A symlink to the given link target
    Link(OsString),
    /// A purely virtual tree containing subtrees
    VirtualTree(BTreeMap<OsString, VfsTree>),
}

impl VfsTree {
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
    /// * [`VfsErrorKind::OnlyNormalPathsAreAllowed`] if the path is not a normal path
    /// * [`VfsErrorKind::DirectoryExistsAsNonVirtual`] if the path is a directory in the repository
    ///
    /// # Returns
    ///
    /// `Ok(())` if the tree was added successfully
    ///
    /// [`VfsErrorKind::DirectoryExistsAsNonVirtual`]: crate::error::VfsErrorKind::DirectoryExistsAsNonVirtual
    /// [`VfsErrorKind::OnlyNormalPathsAreAllowed`]: crate::error::VfsErrorKind::OnlyNormalPathsAreAllowed
    fn add_tree(&mut self, path: &Path, new_tree: Self) -> RusticResult<()> {
        let mut tree = self;
        let mut components = path.components();
        let Component::Normal(last) = components.next_back().unwrap() else {
            return Err(VfsErrorKind::OnlyNormalPathsAreAllowed.into());
        };

        for comp in components {
            if let Component::Normal(name) = comp {
                match tree {
                    Self::VirtualTree(vtree) => {
                        tree = vtree
                            .entry(name.to_os_string())
                            .or_insert(Self::VirtualTree(BTreeMap::new()));
                    }
                    _ => {
                        return Err(VfsErrorKind::DirectoryExistsAsNonVirtual.into());
                    }
                }
            }
        }

        let Self::VirtualTree(vtree) = tree else {
            return Err(VfsErrorKind::DirectoryExistsAsNonVirtual.into());
        };

        _ = vtree.insert(last.to_os_string(), new_tree);
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
    fn get_path(&self, path: &Path) -> RusticResult<(&Self, Option<PathBuf>)> {
        let mut tree = self;
        let mut components = path.components();
        loop {
            match tree {
                Self::RusticTree(_) => {
                    let path: PathBuf = components.collect();
                    return Ok((tree, Some(path)));
                }
                Self::VirtualTree(vtree) => match components.next() {
                    Some(std::path::Component::Normal(name)) => {
                        if let Some(new_tree) = vtree.get(name) {
                            tree = new_tree;
                        } else {
                            return Err(VfsErrorKind::NameDoesNotExist(name.to_os_string()).into());
                        };
                    }
                    None => {
                        return Ok((tree, None));
                    }

                    _ => {}
                },
                Self::Link(_) => return Ok((tree, None)),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, EnumString)]
#[strum(ascii_case_insensitive)]
#[non_exhaustive]
/// Policy to describe how to handle access to a file
pub enum FilePolicy {
    /// Read the file
    Read,
    /// Don't allow reading the file
    Forbidden,
}

#[derive(Debug)]
/// A virtual file system which offers repository contents
pub struct Vfs {
    /// The root tree
    tree: VfsTree,
}

impl Vfs {
    /// Create a new [`Vfs`] from a directory [`Node`].
    pub fn from_dirnode(node: Node) -> Self {
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
    /// * [`VfsErrorKind::OnlyNormalPathsAreAllowed`] if the path is not a normal path
    /// * [`VfsErrorKind::DirectoryExistsAsNonVirtual`] if the path is a directory in the repository
    ///
    /// [`VfsErrorKind::DirectoryExistsAsNonVirtual`]: crate::error::VfsErrorKind::DirectoryExistsAsNonVirtual
    /// [`VfsErrorKind::OnlyNormalPathsAreAllowed`]: crate::error::VfsErrorKind::OnlyNormalPathsAreAllowed
    pub fn from_snapshots(
        mut snapshots: Vec<SnapshotFile>,
        path_template: String,
        time_template: String,
        latest_option: Latest,
        id_snap_option: IdenticalSnapshot,
    ) -> RusticResult<Self> {
        snapshots.sort_unstable();
        let mut tree = VfsTree::new();

        // to handle identical trees
        let mut last_parent = None;
        let mut last_name = None;
        let mut last_tree = Id::default();

        // to handle "latest" entries
        let mut dirs_for_link = BTreeMap::new();
        let mut dirs_for_snap = BTreeMap::new();

        for snap in snapshots {
            let path = FormatArgs::new(
                path_template.as_str(),
                &FormattedSnapshot {
                    snap: &snap,
                    timeformat: &time_template,
                },
            )
            .to_string();
            let path = Path::new(&path);
            let filename = path.file_name().map(OsStr::to_os_string);
            let parent_path = path.parent().map(Path::to_path_buf);

            // Save pathes for latest entries, if requested
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
                        tree.add_tree(path, VfsTree::Link(name))?;
                    }
                } else {
                    tree.add_tree(path, VfsTree::RusticTree(snap.tree))?;
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
                        tree.add_tree(&path, VfsTree::Link(target))?;
                    }
                }
            }
            Latest::AsDir => {
                for (path, subtree) in dirs_for_snap {
                    if let Some(mut path) = path {
                        path.push("latest");
                        tree.add_tree(&path, VfsTree::RusticTree(subtree))?;
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
    /// * [`VfsErrorKind::NameDoesNotExist`] - if the component name doesn't exist
    ///
    /// # Returns
    ///
    /// The [`Node`] at the specified path
    ///
    /// [`VfsErrorKind::NameDoesNotExist`]: crate::error::VfsErrorKind::NameDoesNotExist
    pub fn node_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> RusticResult<Node> {
        let (tree, path) = self.tree.get_path(path)?;
        let meta = Metadata::default();
        match tree {
            VfsTree::RusticTree(tree_id) => Ok(repo.node_from_path(*tree_id, &path.unwrap())?),
            VfsTree::VirtualTree(_) => {
                Ok(Node::new(String::new(), NodeType::Dir, meta, None, None))
            }
            VfsTree::Link(target) => {
                return Ok(Node::new(
                    String::new(),
                    NodeType::from_link(Path::new(target)),
                    meta,
                    None,
                    None,
                ));
            }
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
    /// * [`VfsErrorKind::NameDoesNotExist`] - if the component name doesn't exist
    ///
    /// # Returns
    ///
    /// The list of [`Node`]s at the specified path
    ///
    /// [`VfsErrorKind::NameDoesNotExist`]: crate::error::VfsErrorKind::NameDoesNotExist
    pub fn dir_entries_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> RusticResult<Vec<Node>> {
        let (tree, path) = self.tree.get_path(path)?;

        let result = match tree {
            VfsTree::RusticTree(tree_id) => {
                let node = repo.node_from_path(*tree_id, &path.unwrap())?;
                if node.is_dir() {
                    let tree = repo.get_tree(&node.subtree.unwrap())?;
                    tree.nodes
                } else {
                    Vec::new()
                }
            }
            VfsTree::VirtualTree(vtree) => vtree
                .iter()
                .map(|(name, tree)| {
                    let node_type = match tree {
                        VfsTree::Link(target) => NodeType::from_link(Path::new(target)),
                        _ => NodeType::Dir,
                    };
                    Node::new_node(name, node_type, Metadata::default())
                })
                .collect(),
            VfsTree::Link(str) => {
                return Err(VfsErrorKind::NoDirectoryEntriesForSymlinkFound(str.clone()).into());
            }
        };
        Ok(result)
    }

    #[cfg(feature = "webdav")]
    /// Turn the [`Vfs`] into a [`WebDavFS`]
    pub fn into_webdav_fs<P, S: IndexedFull>(
        self,
        repo: Repository<P, S>,
        file_policy: FilePolicy,
    ) -> Box<WebDavFS<P, S>> {
        WebDavFS::new(repo, self, file_policy)
    }
}

/// `OpenFile` stores all information needed to access the contents of a file node
#[derive(Debug)]
pub struct OpenFile {
    // The list of blobs
    content: Vec<BlobInfo>,
}

// Information about the blob: 1) The id 2) The cumulated sizes of all blobs prior to this one, a.k.a the starting point of this blob.
#[derive(Debug)]
struct BlobInfo {
    // [`Id`] of the blob
    id: Id,
    // the start position of this blob within the file
    starts_at: usize,
}

impl OpenFile {
    /// Create an `OpenFile` from a `Node`
    pub fn from_node<P, S: IndexedFull>(repo: &Repository<P, S>, node: &Node) -> Self {
        let mut start = 0;
        let mut content: Vec<_> = node
            .content
            .as_ref()
            .unwrap()
            .iter()
            .map(|id| {
                let starts_at = start;
                start += repo.index().get_data(id).unwrap().data_length() as usize;
                BlobInfo { id: *id, starts_at }
            })
            .collect();

        // content is assumed to be partioned, so we add a starts_at:MAX entry
        content.push(BlobInfo {
            id: Id::default(),
            starts_at: usize::MAX,
        });

        Self { content }
    }

    /// Read the `OpenFile` at the given `offset` from the `repo`.
    pub fn read_at<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        mut offset: usize,
        mut length: usize,
    ) -> RusticResult<Bytes> {
        // find the start of relevant blobs => find the largest index such that self.content[i].starts_at <= offset, but
        // self.content[i+1] > offset  (note that a last dummy element has been added)
        let mut i = self.content.partition_point(|c| c.starts_at <= offset) - 1;
        offset -= self.content[i].starts_at;
        let mut result = BytesMut::with_capacity(length);

        while length > 0 && i < self.content.len() {
            let data = repo.get_blob_cached(&self.content[i].id, BlobType::Data)?;
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
