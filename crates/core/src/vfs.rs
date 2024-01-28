mod format;
#[cfg(feature = "webdav")]
mod webdavfs;

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail};
use bytes::{Bytes, BytesMut};
use runtime_format::FormatArgs;
use strum::EnumString;

#[cfg(feature = "webdav")]
/// A struct which enables `WebDAV` access to a [`Vfs`] using [`dav-server`]
pub use webdavfs::WebDavFS;

use crate::repofile::{BlobType, Metadata, Node, NodeType, SnapshotFile};
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
enum VfsTree {
    RusticTree(Id),
    Link(OsString),
    VirtualTree(BTreeMap<OsString, VfsTree>),
}

impl VfsTree {
    fn new() -> Self {
        Self::VirtualTree(BTreeMap::new())
    }

    fn add_tree(&mut self, path: &Path, new_tree: Self) -> anyhow::Result<()> {
        let mut tree = self;
        let mut components = path.components();
        let Component::Normal(last) = components.next_back().unwrap() else {
            bail!("only normal paths allowed!");
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
                        bail!("dir exists as non-virtual dir")
                    }
                }
            }
        }

        let Self::VirtualTree(vtree) = tree else {
            bail!("dir exists as non-virtual dir!")
        };

        _ = vtree.insert(last.to_os_string(), new_tree);
        Ok(())
    }

    fn get_path(&self, path: &Path) -> anyhow::Result<(&Self, Option<PathBuf>)> {
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
                        tree = vtree
                            .get(name)
                            .ok_or_else(|| anyhow!("name {:?} doesn't exist", name))?;
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
/// Policy to describe how to handle access to a file within the [`Vfs`]
pub enum FilePolicy {
    /// Read the file
    Read,
    /// Don't allow reading the file
    Forbidden,
}

#[derive(Debug)]
/// A virtual file system which offers repository contents
pub struct Vfs {
    tree: VfsTree,
    file_policy: FilePolicy,
}

impl Vfs {
    /// Create a new [`Vfs`] from a directory [`Node`].
    pub fn from_dirnode(node: Node, file_policy: FilePolicy) -> Self {
        let tree = VfsTree::RusticTree(node.subtree.unwrap());
        Self { tree, file_policy }
    }

    /// Create a new [`Vfs`] from a list of snapshots.
    pub fn from_snapshots(
        mut snapshots: Vec<SnapshotFile>,
        path_template: String,
        time_template: String,
        latest_option: Latest,
        id_snap_option: IdenticalSnapshot,
        file_policy: FilePolicy,
    ) -> anyhow::Result<Self> {
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
        Ok(Self { tree, file_policy })
    }

    /// Get a [`Node`] from the specified path.
    pub fn node_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> anyhow::Result<Node> {
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
    pub fn dir_entries_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> anyhow::Result<Vec<Node>> {
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
            VfsTree::Link(_) => {
                bail!("no dir entries for symlink!");
            }
        };
        Ok(result)
    }

    #[cfg(feature = "webdav")]
    /// Turn the [`Vfs`] into a [`WebDavFS`]
    pub fn into_webdav_fs<P, S: IndexedFull>(self, repo: Repository<P, S>) -> Box<WebDavFS<P, S>> {
        WebDavFS::new(repo, self)
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
    id: Id,
    cumsize: usize,
}

impl OpenFile {
    /// Create an `OpenFile` from a `Node`
    pub fn from_node<P, S: IndexedFull>(repo: &Repository<P, S>, node: &Node) -> Self {
        let mut start = 0;
        let content = node
            .content
            .as_ref()
            .unwrap()
            .iter()
            .map(|id| {
                let cumsize = start;
                start += repo.index().get_data(id).unwrap().data_length() as usize;
                BlobInfo { id: *id, cumsize }
            })
            .collect();

        Self { content }
    }

    /// Read the `OpenFile` at the given `offset` from the `repo`.
    pub fn read_at<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        mut offset: usize,
        mut length: usize,
    ) -> RusticResult<Bytes> {
        // find the start of relevant blobs
        let mut i = self.content.partition_point(|c| c.cumsize <= offset) - 1;
        offset -= self.content[i].cumsize;

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
