use std::{
    cmp::Ordering,
    ffi::{OsStr, OsString},
};

use log::warn;

use crate::{
    archiver::{TreeStackEmptyError, tree::TreeType},
    backend::{decrypt::DecryptReadBackend, node::Node},
    blob::tree::{Tree, TreeId},
    index::ReadGlobalIndex,
};

/// The `ItemWithParent` is a `TreeType` wrapping the result of a parent search and a type `O`.
///
/// # Type Parameters
///
/// * `O` - The type of the `TreeType`.
pub(crate) type ItemWithParent<O> = TreeType<(O, ParentResult<()>), ParentResult<TreeId>>;

/// The `Parent` is responsible for finding the parent tree of a given tree.
#[derive(Debug)]
pub struct Parent {
    /// The tree id of the parent tree.
    tree_ids: Vec<TreeId>,
    /// The parent tree.
    trees: Vec<(Tree, usize)>,
    /// The stack of parent trees.
    stack: Vec<Vec<(Tree, usize)>>,
    /// Ignore ctime when comparing nodes.
    ignore_ctime: bool,
    /// Ignore inode number when comparing nodes.
    ignore_inode: bool,
}

/// The result of a parent search.
///
/// # Type Parameters
///
/// * `T` - The type of the matched parent.
#[derive(Clone, Debug)]
pub(crate) enum ParentResult<T> {
    /// The parent was found and matches.
    Matched(T),
    /// The parent was not found.
    NotFound,
    /// The parent was found but doesn't match.
    NotMatched,
}

impl<T> ParentResult<T> {
    /// Maps a `ParentResult<T>` to a `ParentResult<R>` by applying a function to a contained value.
    ///
    /// # Type Parameters
    ///
    /// * `R` - The type of the returned `ParentResult`.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to apply.
    ///
    /// # Returns
    ///
    /// A `ParentResult<R>` with the result of the function for each `ParentResult<T>`.
    fn map<R>(self, f: impl FnOnce(T) -> R) -> ParentResult<R> {
        match self {
            Self::Matched(t) => ParentResult::Matched(f(t)),
            Self::NotFound => ParentResult::NotFound,
            Self::NotMatched => ParentResult::NotMatched,
        }
    }
}

impl Parent {
    /// Creates a new `Parent`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The type of the backend.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `tree_id` - The tree id of the parent tree.
    /// * `ignore_ctime` - Ignore ctime when comparing nodes.
    /// * `ignore_inode` - Ignore inode number when comparing nodes.
    pub(crate) fn new(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        tree_id: impl IntoIterator<Item = TreeId>,
        ignore_ctime: bool,
        ignore_inode: bool,
    ) -> Self {
        // if tree_id is given, try to load tree from backend.
        let (trees, tree_ids) = tree_id
            .into_iter()
            .filter_map(|tree_id| match Tree::from_backend(be, index, tree_id) {
                Ok(tree) => Some(((tree, 0), tree_id)),
                Err(err) => {
                    warn!(
                        "ignoring error when loading parent tree {tree_id:?}: {}",
                        err.display_log()
                    );
                    None
                }
            })
            .unzip();

        Self {
            tree_ids,
            trees,
            stack: Vec::new(),
            ignore_ctime,
            ignore_inode,
        }
    }

    /// Returns the parent node with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the parent node.
    ///
    /// # Returns
    ///
    /// The parent node with the given name, or `None` if the parent node is not found.
    fn p_node(&mut self, name: &OsStr) -> impl Iterator<Item = &Node> {
        self.trees.iter_mut().filter_map(|(tree, idx)| {
            let p_nodes = &tree.nodes;
            loop {
                match p_nodes.get(*idx) {
                    None => break None,
                    Some(p_node) => match p_node.name().as_os_str().cmp(name) {
                        Ordering::Less => *idx += 1,
                        Ordering::Equal => {
                            break Some(p_node);
                        }
                        Ordering::Greater => {
                            break None;
                        }
                    },
                }
            }
        })
    }

    /// Returns whether the given node is the parent of the given tree.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to check.
    /// * `name` - The name of the tree.
    ///
    /// # Returns
    ///
    /// Whether the given node is the parent of the given tree.
    ///
    /// # Note
    ///
    /// TODO: This function does not check whether the given node is a directory.
    fn is_parent(&mut self, node: &Node, name: &OsStr) -> ParentResult<&Node> {
        // use new variables as the mutable borrow is used later
        let ignore_ctime = self.ignore_ctime;
        let ignore_inode = self.ignore_inode;

        let mut p_node = self.p_node(name).peekable();
        if p_node.peek().is_none() {
            return ParentResult::NotFound;
        }

        p_node
            .find(|p_node| {
                p_node.node_type == node.node_type
                    && p_node.meta.size == node.meta.size
                    && p_node.meta.mtime == node.meta.mtime
                    && (ignore_ctime || p_node.meta.ctime == node.meta.ctime)
                    && (ignore_inode
                        || p_node.meta.inode == 0
                        || p_node.meta.inode == node.meta.inode)
            })
            .map_or(ParentResult::NotMatched, ParentResult::Matched)
    }

    // TODO: add documentation!
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The type of the backend.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `name` - The name of the parent node.
    fn set_dir(
        &mut self,
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        name: &OsStr,
    ) {
        let mut new_ids: Vec<_> = self
            .p_node(name)
            .filter_map(|p_node| {
                p_node.subtree.or_else(|| {
                    warn!("ignoring parent node {}: is no tree!", p_node.name);
                    None
                })
            })
            .collect();

        // remove potentially identical trees
        new_ids.sort();
        new_ids.dedup();

        let new_tree = new_ids
            .into_iter()
            .filter_map(|tree_id| match Tree::from_backend(be, index, tree_id) {
                Ok(tree) => Some((tree, 0)),
                Err(err) => {
                    warn!(
                        "ignoring error when loading parent tree {tree_id}: {}",
                        err.display_log()
                    );
                    None
                }
            })
            .collect();
        let old_tree = std::mem::replace(&mut self.trees, new_tree);
        self.stack.push(old_tree);
    }

    // TODO: add documentation!
    ///
    /// # Errors
    ///
    /// * If the tree stack is empty.
    fn finish_dir(&mut self) -> Result<(), TreeStackEmptyError> {
        let tree = self.stack.pop().ok_or(TreeStackEmptyError)?;
        self.trees = tree;

        Ok(())
    }

    // TODO: add documentation!
    pub(crate) fn tree_id(&self) -> Option<TreeId> {
        self.tree_ids.first().copied()
    }

    // TODO: add documentation!
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The type of the backend.
    /// * `O` - The type of the tree item.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `item` - The item to process.
    ///
    /// # Errors
    ///
    /// * If the tree stack is empty.
    pub(crate) fn process<O>(
        &mut self,
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        item: TreeType<O, OsString>,
    ) -> Result<ItemWithParent<O>, TreeStackEmptyError> {
        let result = match item {
            TreeType::NewTree((path, node, tree)) => {
                let parent_result = self
                    .is_parent(&node, &tree)
                    .map(|node| node.subtree.unwrap());
                self.set_dir(be, index, &tree);
                TreeType::NewTree((path, node, parent_result))
            }
            TreeType::EndTree => {
                self.finish_dir()?;
                TreeType::EndTree
            }
            TreeType::Other((path, mut node, open)) => {
                let parent = self.is_parent(&node, &node.name());
                let parent = match parent {
                    ParentResult::Matched(p_node) => {
                        if p_node.content.iter().flatten().all(|id| index.has_data(id)) {
                            node.content.clone_from(&p_node.content);
                            ParentResult::Matched(())
                        } else {
                            warn!(
                                "missing blobs in index for unchanged file {}; re-reading file",
                                path.display()
                            );
                            ParentResult::NotFound
                        }
                    }
                    parent_result => parent_result.map(|_| ()),
                };
                TreeType::Other((path, node, (open, parent)))
            }
        };
        Ok(result)
    }
}
