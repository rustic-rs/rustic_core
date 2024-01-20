mod format;
#[cfg(feature = "webdav")]
mod webdavfs;

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail};
use runtime_format::FormatArgs;

#[cfg(feature = "webdav")]
pub use webdavfs::WebDavFS;

use crate::repofile::{Metadata, Node, NodeType, SnapshotFile};
use crate::{Id, IndexedFull, Repository};

use format::FormattedSnapshot;

#[derive(Debug, Clone, Copy)]
pub enum IdenticalSnapshot {
    AsLink,
    AsDir,
}

#[derive(Debug, Clone, Copy)]
pub enum Latest {
    No,
    AsLink,
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

#[derive(Debug)]
pub struct Vfs(VfsTree);

impl Vfs {
    pub fn new() -> Self {
        Self(VfsTree::new())
    }

    pub fn from_dirnode(node: Node) -> Self {
        Vfs(VfsTree::RusticTree(node.subtree.unwrap()))
    }

    pub fn from_snapshots(
        mut snapshots: Vec<SnapshotFile>,
        path_template: String,
        time_template: String,
        latest_option: Latest,
        id_snap_option: IdenticalSnapshot,
    ) -> anyhow::Result<Self> {
        snapshots.sort_unstable();
        let mut root = VfsTree::new();

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
                        root.add_tree(path, VfsTree::Link(name))?;
                    }
                } else {
                    root.add_tree(path, VfsTree::RusticTree(snap.tree))?;
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
                        root.add_tree(&path, VfsTree::Link(target))?;
                    }
                }
            }
            Latest::AsDir => {
                for (path, tree) in dirs_for_snap {
                    if let Some(mut path) = path {
                        path.push("latest");
                        root.add_tree(&path, VfsTree::RusticTree(tree))?;
                    }
                }
            }
        }
        Ok(Self(root))
    }

    pub fn node_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> anyhow::Result<Node> {
        let (tree, path) = self.0.get_path(path)?;
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

    pub fn dir_entries_from_path<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        path: &Path,
    ) -> anyhow::Result<Vec<Node>> {
        let (tree, path) = self.0.get_path(path)?;

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
    pub fn into_webdav_fs<P, S: IndexedFull>(self, repo: Repository<P, S>) -> Box<WebDavFS<P, S>> {
        WebDavFS::new(repo, self)
    }
}
