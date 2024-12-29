use crate::backend::node::{Metadata, Node, NodeType};

use typed_path::{Component, UnixPathBuf};

/// `TreeIterator` turns an Iterator yielding items with paths and Nodes into an
/// Iterator which ensures that all subdirectories are visited and closed.
/// The resulting Iterator yielss a `TreeType` which either contains the original
/// item, a new tree to be inserted or a pseudo item which indicates that a tree is finished.
///
/// # Type Parameters
///
/// * `T` - The type of the current item.
/// * `I` - The type of the original Iterator.
pub(crate) struct TreeIterator<T, I> {
    /// The original Iterator.
    iter: I,
    /// The current path.
    path: UnixPathBuf,
    /// The current item.
    item: Option<T>,
}

impl<T, I> TreeIterator<T, I>
where
    I: Iterator<Item = T>,
{
    pub(crate) fn new(mut iter: I) -> Self {
        let item = iter.next();
        Self {
            iter,
            path: UnixPathBuf::new(),
            item,
        }
    }
}

/// `TreeType` is the type returned by the `TreeIterator`.
///
/// It either contains the original item, a new tree to be inserted
/// or a pseudo item which indicates that a tree is finished.
///
/// # Type Parameters
///
/// * `T` - The type of the original item.
/// * `U` - The type of the new tree.
#[derive(Debug)]
pub(crate) enum TreeType<T, U> {
    /// New tree to be inserted.
    NewTree((UnixPathBuf, Node, U)),
    /// A pseudo item which indicates that a tree is finished.
    EndTree,
    /// Original item.
    Other((UnixPathBuf, Node, T)),
}

impl<I, O> Iterator for TreeIterator<(UnixPathBuf, Node, O), I>
where
    I: Iterator<Item = (UnixPathBuf, Node, O)>,
{
    type Item = TreeType<O, Vec<u8>>;
    fn next(&mut self) -> Option<Self::Item> {
        match &self.item {
            None => {
                if self.path.pop() {
                    Some(TreeType::EndTree)
                } else {
                    None
                }
            }
            Some((path, node, _)) => {
                match path.strip_prefix(&self.path) {
                    Err(_) => {
                        _ = self.path.pop();
                        Some(TreeType::EndTree)
                    }
                    Ok(missing_dirs) => {
                        if let Some(p) = missing_dirs.components().next() {
                            self.path.push(p);
                            if node.is_dir() && path == &self.path {
                                let (path, node, _) = self.item.take().unwrap();
                                self.item = self.iter.next();
                                let name = node.name().to_vec();
                                return Some(TreeType::NewTree((path, node, name)));
                            }
                            // Use mode 755 for missing dirs, so they can be accessed
                            let meta = Metadata {
                                mode: Some(0o755),
                                ..Default::default()
                            };
                            let node = Node::new_node(p.as_bytes(), NodeType::Dir, meta);
                            return Some(TreeType::NewTree((
                                self.path.clone(),
                                node,
                                p.as_bytes().to_vec(),
                            )));
                        }
                        // there wasn't any normal path component to process - return current item
                        let item = self.item.take().unwrap();
                        self.item = self.iter.next();
                        Some(TreeType::Other(item))
                    }
                }
            }
        }
    }
}
