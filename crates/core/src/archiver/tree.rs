use std::{
    ffi::OsString,
    path::{Component, PathBuf},
};

use crate::{
    backend::node::{Metadata, Node, NodeType},
    blob::tree::comp_to_osstr,
};

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
    path: PathBuf,
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
            path: PathBuf::new(),
            item,
        }
    }

    // like self.path.pop(), but does also pop path prefixes (on windows; like "C:\")
    fn pop(&mut self) -> bool {
        let mut comps = self.path.components();
        loop {
            let comp = comps.next_back();
            match comp {
                Some(Component::Prefix(_) | Component::Normal(_)) => {
                    self.path = comps.collect();
                    return true;
                }
                Some(Component::RootDir | Component::ParentDir | Component::CurDir) => {}
                None => return false,
            }
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
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TreeType<T, U> {
    /// New tree to be inserted.
    NewTree((PathBuf, Node, U)),
    /// A pseudo item which indicates that a tree is finished.
    EndTree,
    /// Original item.
    Other((PathBuf, Node, T)),
}

impl<I, O> Iterator for TreeIterator<(PathBuf, Node, O), I>
where
    I: Iterator<Item = (PathBuf, Node, O)>,
{
    type Item = TreeType<O, OsString>;
    fn next(&mut self) -> Option<Self::Item> {
        match &self.item {
            None => self.pop().then_some(TreeType::EndTree),
            Some((path, node, _)) => {
                match path.strip_prefix(&self.path) {
                    Err(_) => {
                        _ = self.pop();
                        Some(TreeType::EndTree)
                    }
                    Ok(missing_dirs) => {
                        for comp in missing_dirs.components() {
                            self.path.push(comp);
                            // process next normal path component - other components are simply ignored
                            if let Some(p) = comp_to_osstr(comp).ok().flatten() {
                                if node.is_dir() && path == &self.path {
                                    let (path, node, _) = self.item.take().unwrap();
                                    self.item = self.iter.next();
                                    let name = node.name();
                                    return Some(TreeType::NewTree((path, node, name)));
                                }
                                // Use mode 755 for missing dirs, so they can be accessed
                                let meta = Metadata {
                                    mode: Some(0o755),
                                    ..Default::default()
                                };
                                let node = Node::new_node(&p, NodeType::Dir, meta);
                                return Some(TreeType::NewTree((self.path.clone(), node, p)));
                            }
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

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;
    use rstest::rstest;

    use super::*;
    use std::path::Path;

    fn test_tree_iter(case: &str, paths: Vec<&str>) {
        let paths = paths.into_iter().map(|p| {
            (
                Path::new(&p).to_path_buf(),
                Node::new_node(
                    Path::new(&p).file_name().unwrap(),
                    NodeType::Dir,
                    Metadata::default(),
                ),
                (),
            )
        });

        let result: Vec<_> = TreeIterator::new(paths).collect();
        assert_debug_snapshot!(format!("tree_iter#{case}"), result);
    }

    #[cfg(not(windows))]
    #[rstest]
    #[case("simple", ["a", "a/b", "a/b/c"].to_vec())]
    #[case("simple_root", ["/a", "/a/b", "/a/b/c"].to_vec())]
    #[case("simple_relative", ["./a", "./a/b", "./a/b/c"].to_vec())]
    #[case("complex", ["a/b", "a/b/c", "a/b/d", "f"].to_vec())]
    #[case("complex_root", ["/a/b", "/a/b/c", "/a/b/d", "/f"].to_vec())]
    #[case("complex_relative", ["./a/b", "./a/b/c", "./a/b/d", "./f"].to_vec())]
    fn test_tree_iter_nix(#[case] case: &str, #[case] paths: Vec<&str>) {
        test_tree_iter(case, paths);
    }

    #[cfg(windows)]
    #[rstest]
    #[case("windows", [r"C:\a", r"C:\a\b", r"D:\a"].to_vec())]
    fn test_tree_iter_windows(#[case] case: &str, #[case] paths: Vec<&str>) {
        test_tree_iter(case, paths);
    }
}
