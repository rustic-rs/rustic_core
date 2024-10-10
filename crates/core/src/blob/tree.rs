use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    ffi::{OsStr, OsString},
    mem,
    path::{Component, Path, PathBuf, Prefix},
    str,
};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use derivative::Derivative;
use derive_setters::Setters;
use ignore::overrides::{Override, OverrideBuilder};
use ignore::Match;

use serde::{Deserialize, Deserializer};
use serde_derive::Serialize;

use crate::{
    backend::{
        decrypt::DecryptReadBackend,
        node::{Metadata, Node, NodeType},
    },
    blob::BlobType,
    crypto::hasher::hash,
    error::{RusticResult, TreeErrorKind},
    impl_blobid,
    index::ReadGlobalIndex,
    progress::Progress,
    repofile::snapshotfile::SnapshotSummary,
};

pub(super) mod constants {
    /// The maximum number of trees that are loaded in parallel
    pub(super) const MAX_TREE_LOADER: usize = 4;
}

pub(crate) type TreeStreamItem = RusticResult<(PathBuf, Tree)>;
type NodeStreamItem = RusticResult<(PathBuf, Node)>;
impl_blobid!(TreeId, BlobType::Tree);

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
/// A [`Tree`] is a list of [`Node`]s
pub struct Tree {
    #[serde(deserialize_with = "deserialize_null_default")]
    /// The nodes contained in the tree.
    ///
    /// This is usually sorted by `Node.name()`, i.e. by the node name as `OsString`
    pub nodes: Vec<Node>,
}

/// Deserializes `Option<T>` as `T::default()` if the value is `null`
// TODO: Use serde_with::DefaultOnNull instead. But this has problems with RON which is used in our integration tests...
pub(crate) fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

impl Tree {
    /// Creates a new `Tree` with no nodes.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Adds a node to the tree.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to add.
    pub(crate) fn add(&mut self, node: Node) {
        self.nodes.push(node);
    }

    /// Serializes the tree.
    ///
    /// # Returns
    ///
    /// A tuple of the serialized tree as `Vec<u8>` and the tree's ID
    pub(crate) fn serialize(&self) -> RusticResult<(Vec<u8>, TreeId)> {
        let mut chunk = serde_json::to_vec(&self).map_err(TreeErrorKind::SerializingTreeFailed)?;
        chunk.push(b'\n'); // for whatever reason, restic adds a newline, so to be compatible...
        let id = hash(&chunk).into();
        Ok((chunk, id))
    }

    /// Deserializes a tree from the backend.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `id` - The ID of the tree to deserialize.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::BlobIdNotFound`] - If the tree ID is not found in the backend.
    /// * [`TreeErrorKind::DeserializingTreeFailed`] - If deserialization fails.
    ///
    /// # Returns
    ///
    /// The deserialized tree.
    ///
    /// [`TreeErrorKind::BlobIdNotFound`]: crate::error::TreeErrorKind::BlobIdNotFound
    /// [`TreeErrorKind::DeserializingTreeFailed`]: crate::error::TreeErrorKind::DeserializingTreeFailed
    pub(crate) fn from_backend(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        id: TreeId,
    ) -> RusticResult<Self> {
        let data = index
            .get_tree(&id)
            .ok_or_else(|| TreeErrorKind::BlobIdNotFound(id))?
            .read_data(be)?;

        Ok(serde_json::from_slice(&data).map_err(TreeErrorKind::DeserializingTreeFailed)?)
    }

    /// Creates a new node from a path.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `id` - The ID of the tree to deserialize.
    /// * `path` - The path to create the node from.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::NotADirectory`] - If the path is not a directory.
    /// * [`TreeErrorKind::PathNotFound`] - If the path is not found.
    /// * [`TreeErrorKind::PathIsNotUtf8Conform`] - If the path is not UTF-8 conform.
    ///
    /// [`TreeErrorKind::NotADirectory`]: crate::error::TreeErrorKind::NotADirectory
    /// [`TreeErrorKind::PathNotFound`]: crate::error::TreeErrorKind::PathNotFound
    /// [`TreeErrorKind::PathIsNotUtf8Conform`]: crate::error::TreeErrorKind::PathIsNotUtf8Conform
    pub(crate) fn node_from_path(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        id: TreeId,
        path: &Path,
    ) -> RusticResult<Node> {
        let mut node = Node::new_node(OsStr::new(""), NodeType::Dir, Metadata::default());
        node.subtree = Some(id);

        for p in path.components() {
            if let Some(p) = comp_to_osstr(p)? {
                let id = node
                    .subtree
                    .ok_or_else(|| TreeErrorKind::NotADirectory(p.clone()))?;
                let tree = Self::from_backend(be, index, id)?;
                node = tree
                    .nodes
                    .into_iter()
                    .find(|node| node.name() == p)
                    .ok_or_else(|| TreeErrorKind::PathNotFound(p.clone()))?;
            }
        }

        Ok(node)
    }

    pub(crate) fn find_nodes_from_path(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        ids: impl IntoIterator<Item = TreeId>,
        path: &Path,
    ) -> RusticResult<FindNode> {
        // helper function which is recursively called
        fn find_node_from_component(
            be: &impl DecryptReadBackend,
            index: &impl ReadGlobalIndex,
            tree_id: TreeId,
            path_comp: &[OsString],
            results_cache: &mut [BTreeMap<TreeId, Option<usize>>],
            nodes: &mut BTreeMap<Node, usize>,
            idx: usize,
        ) -> RusticResult<Option<usize>> {
            if let Some(result) = results_cache[idx].get(&tree_id) {
                return Ok(*result);
            }

            let tree = Tree::from_backend(be, index, tree_id)?;
            let result = if let Some(node) = tree
                .nodes
                .into_iter()
                .find(|node| node.name() == path_comp[idx])
            {
                if idx == path_comp.len() - 1 {
                    let new_idx = nodes.len();
                    let node_idx = nodes.entry(node).or_insert(new_idx);
                    Some(*node_idx)
                } else {
                    let id = node
                        .subtree
                        .ok_or_else(|| TreeErrorKind::NotADirectory(path_comp[idx].clone()))?;

                    find_node_from_component(
                        be,
                        index,
                        id,
                        path_comp,
                        results_cache,
                        nodes,
                        idx + 1,
                    )?
                }
            } else {
                None
            };
            _ = results_cache[idx].insert(tree_id, result);
            Ok(result)
        }

        let path_comp: Vec<_> = path
            .components()
            .filter_map(|p| comp_to_osstr(p).transpose())
            .collect::<RusticResult<_>>()?;

        // caching all results
        let mut results_cache = vec![BTreeMap::new(); path_comp.len()];
        let mut nodes = BTreeMap::new();

        let matches: Vec<_> = ids
            .into_iter()
            .map(|id| {
                find_node_from_component(
                    be,
                    index,
                    id,
                    &path_comp,
                    &mut results_cache,
                    &mut nodes,
                    0,
                )
            })
            .collect::<RusticResult<_>>()?;

        // sort nodes by index and return a Vec
        let mut nodes: Vec<_> = nodes.into_iter().collect();
        nodes.sort_unstable_by_key(|n| n.1);
        let nodes = nodes.into_iter().map(|n| n.0).collect();

        Ok(FindNode { nodes, matches })
    }

    pub(crate) fn find_matching_nodes(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        ids: impl IntoIterator<Item = TreeId>,
        matches: &impl Fn(&Path, &Node) -> bool,
    ) -> RusticResult<FindMatches> {
        // internal state used to save match information in find_matching_nodes
        #[derive(Default)]
        struct MatchInternalState {
            // we cache all results
            cache: BTreeMap<(TreeId, PathBuf), Vec<(usize, usize)>>,
            nodes: BTreeMap<Node, usize>,
            paths: BTreeMap<PathBuf, usize>,
        }

        impl MatchInternalState {
            fn insert_result(&mut self, path: PathBuf, node: Node) -> (usize, usize) {
                let new_idx = self.nodes.len();
                let node_idx = self.nodes.entry(node).or_insert(new_idx);
                let new_idx = self.paths.len();
                let node_path_idx = self.paths.entry(path).or_insert(new_idx);
                (*node_path_idx, *node_idx)
            }
        }

        // helper function which is recursively called
        fn find_matching_nodes_recursive(
            be: &impl DecryptReadBackend,
            index: &impl ReadGlobalIndex,
            tree_id: TreeId,
            path: &Path,
            state: &mut MatchInternalState,
            matches: &impl Fn(&Path, &Node) -> bool,
        ) -> RusticResult<Vec<(usize, usize)>> {
            let mut result = Vec::new();
            if let Some(result) = state.cache.get(&(tree_id, path.to_path_buf())) {
                return Ok(result.clone());
            }

            let tree = Tree::from_backend(be, index, tree_id)?;
            for node in tree.nodes {
                let node_path = path.join(node.name());
                if node.is_dir() {
                    let id = node
                        .subtree
                        .ok_or_else(|| TreeErrorKind::NotADirectory(node.name()))?;
                    result.append(&mut find_matching_nodes_recursive(
                        be, index, id, &node_path, state, matches,
                    )?);
                }
                if matches(&node_path, &node) {
                    result.push(state.insert_result(node_path, node));
                }
            }
            _ = state
                .cache
                .insert((tree_id, path.to_path_buf()), result.clone());
            Ok(result)
        }

        let mut state = MatchInternalState::default();

        let initial_path = PathBuf::new();
        let matches: Vec<_> = ids
            .into_iter()
            .map(|id| {
                find_matching_nodes_recursive(be, index, id, &initial_path, &mut state, matches)
            })
            .collect::<RusticResult<_>>()?;

        // sort paths by index and return a Vec
        let mut paths: Vec<_> = state.paths.into_iter().collect();
        paths.sort_unstable_by_key(|n| n.1);
        let paths = paths.into_iter().map(|n| n.0).collect();

        // sort nodes by index and return a Vec
        let mut nodes: Vec<_> = state.nodes.into_iter().collect();
        nodes.sort_unstable_by_key(|n| n.1);
        let nodes = nodes.into_iter().map(|n| n.0).collect();
        Ok(FindMatches {
            paths,
            nodes,
            matches,
        })
    }
}

/// Results from `find_node_from_path`
#[derive(Debug, Serialize)]
pub struct FindNode {
    /// found nodes for the given path
    pub nodes: Vec<Node>,
    /// found nodes for all given snapshots. usize is the index of the node
    pub matches: Vec<Option<usize>>,
}

/// Results from `find_matching_nodes`
#[derive(Debug, Serialize)]
pub struct FindMatches {
    /// found matching paths
    pub paths: Vec<PathBuf>,
    /// found matching nodes
    pub nodes: Vec<Node>,
    /// found paths/nodes for all given snapshots. (usize,usize) is the path / node index
    pub matches: Vec<Vec<(usize, usize)>>,
}

/// Converts a [`Component`] to an [`OsString`].
///
/// # Arguments
///
/// * `p` - The component to convert.
///
/// # Errors
///
/// * [`TreeErrorKind::ContainsCurrentOrParentDirectory`] - If the component is a current or parent directory.
/// * [`TreeErrorKind::PathIsNotUtf8Conform`] - If the component is not UTF-8 conform.
///
/// [`TreeErrorKind::ContainsCurrentOrParentDirectory`]: crate::error::TreeErrorKind::ContainsCurrentOrParentDirectory
/// [`TreeErrorKind::PathIsNotUtf8Conform`]: crate::error::TreeErrorKind::PathIsNotUtf8Conform
pub(crate) fn comp_to_osstr(p: Component<'_>) -> RusticResult<Option<OsString>> {
    let s = match p {
        Component::RootDir => None,
        Component::Prefix(p) => match p.kind() {
            Prefix::Verbatim(p) | Prefix::DeviceNS(p) => Some(p.to_os_string()),
            Prefix::VerbatimUNC(_, q) | Prefix::UNC(_, q) => Some(q.to_os_string()),
            Prefix::VerbatimDisk(p) | Prefix::Disk(p) => Some(
                OsStr::new(str::from_utf8(&[p]).map_err(TreeErrorKind::PathIsNotUtf8Conform)?)
                    .to_os_string(),
            ),
        },
        Component::Normal(p) => Some(p.to_os_string()),
        _ => return Err(TreeErrorKind::ContainsCurrentOrParentDirectory.into()),
    };
    Ok(s)
}

impl IntoIterator for Tree {
    type Item = Node;
    type IntoIter = std::vec::IntoIter<Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.nodes.into_iter()
    }
}

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Derivative, Clone, Debug, Setters)]
#[derivative(Default)]
#[setters(into)]
#[non_exhaustive]
/// Options for listing the `Nodes` of a `Tree`
pub struct TreeStreamerOptions {
    /// Glob pattern to exclude/include (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long, help_heading = "Exclude options"))]
    pub glob: Vec<String>,

    /// Same as --glob pattern but ignores the casing of filenames
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "GLOB", help_heading = "Exclude options")
    )]
    pub iglob: Vec<String>,

    /// Read glob patterns to exclude/include from this file (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "FILE", help_heading = "Exclude options")
    )]
    pub glob_file: Vec<String>,

    /// Same as --glob-file ignores the casing of filenames in patterns
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "FILE", help_heading = "Exclude options")
    )]
    pub iglob_file: Vec<String>,

    /// recursively list the dir
    #[cfg_attr(feature = "clap", clap(long))]
    #[derivative(Default(value = "true"))]
    pub recursive: bool,
}

/// [`NodeStreamer`] recursively streams all nodes of a given tree including all subtrees in-order
#[derive(Debug, Clone)]
pub struct NodeStreamer<'a, BE, I>
where
    BE: DecryptReadBackend,
    I: ReadGlobalIndex,
{
    /// The open iterators for subtrees
    open_iterators: Vec<std::vec::IntoIter<Node>>,
    /// Inner iterator for the current subtree nodes
    inner: std::vec::IntoIter<Node>,
    /// The current path
    path: PathBuf,
    /// The backend to read from
    be: BE,
    /// index
    index: &'a I,
    /// The glob overrides
    overrides: Option<Override>,
    /// Whether to stream recursively
    recursive: bool,
}

impl<'a, BE, I> NodeStreamer<'a, BE, I>
where
    BE: DecryptReadBackend,
    I: ReadGlobalIndex,
{
    /// Creates a new `NodeStreamer`.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `node` - The node to start from.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::BlobIdNotFound`] - If the tree ID is not found in the backend.
    /// * [`TreeErrorKind::DeserializingTreeFailed`] - If deserialization fails.
    ///
    /// [`TreeErrorKind::BlobIdNotFound`]: crate::error::TreeErrorKind::BlobIdNotFound
    /// [`TreeErrorKind::DeserializingTreeFailed`]: crate::error::TreeErrorKind::DeserializingTreeFailed
    #[allow(unused)]
    pub fn new(be: BE, index: &'a I, node: &Node) -> RusticResult<Self> {
        Self::new_streamer(be, index, node, None, true)
    }

    /// Creates a new `NodeStreamer`.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `node` - The node to start from.
    /// * `overrides` - The glob overrides.
    /// * `recursive` - Whether to stream recursively.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::BlobIdNotFound`] - If the tree ID is not found in the backend.
    /// * [`TreeErrorKind::DeserializingTreeFailed`] - If deserialization fails.
    ///
    /// [`TreeErrorKind::BlobIdNotFound`]: crate::error::TreeErrorKind::BlobIdNotFound
    /// [`TreeErrorKind::DeserializingTreeFailed`]: crate::error::TreeErrorKind::DeserializingTreeFailed
    fn new_streamer(
        be: BE,
        index: &'a I,
        node: &Node,
        overrides: Option<Override>,
        recursive: bool,
    ) -> RusticResult<Self> {
        let inner = if node.is_dir() {
            Tree::from_backend(&be, index, node.subtree.unwrap())?
                .nodes
                .into_iter()
        } else {
            vec![node.clone()].into_iter()
        };
        Ok(Self {
            inner,
            open_iterators: Vec::new(),
            path: PathBuf::new(),
            be,
            index,
            overrides,
            recursive,
        })
    }
    /// Creates a new `NodeStreamer` with glob patterns.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `node` - The node to start from.
    /// * `opts` - The options for the streamer.
    /// * `recursive` - Whether to stream recursively.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::BuildingNodeStreamerFailed`] - If building the streamer fails.
    /// * [`TreeErrorKind::ReadingFileStringFromGlobsFailed`] - If reading a glob file fails.
    ///
    /// [`TreeErrorKind::BuildingNodeStreamerFailed`]: crate::error::TreeErrorKind::BuildingNodeStreamerFailed
    /// [`TreeErrorKind::ReadingFileStringFromGlobsFailed`]: crate::error::TreeErrorKind::ReadingFileStringFromGlobsFailed
    pub fn new_with_glob(
        be: BE,
        index: &'a I,
        node: &Node,
        opts: &TreeStreamerOptions,
    ) -> RusticResult<Self> {
        let mut override_builder = OverrideBuilder::new("");

        for g in &opts.glob {
            _ = override_builder
                .add(g)
                .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;
        }

        for file in &opts.glob_file {
            for line in std::fs::read_to_string(file)
                .map_err(TreeErrorKind::ReadingFileStringFromGlobsFailed)?
                .lines()
            {
                _ = override_builder
                    .add(line)
                    .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;
            }
        }

        _ = override_builder
            .case_insensitive(true)
            .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;
        for g in &opts.iglob {
            _ = override_builder
                .add(g)
                .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;
        }

        for file in &opts.iglob_file {
            for line in std::fs::read_to_string(file)
                .map_err(TreeErrorKind::ReadingFileStringFromGlobsFailed)?
                .lines()
            {
                _ = override_builder
                    .add(line)
                    .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;
            }
        }
        let overrides = override_builder
            .build()
            .map_err(TreeErrorKind::BuildingNodeStreamerFailed)?;

        Self::new_streamer(be, index, node, Some(overrides), opts.recursive)
    }
}

// TODO: This is not parallel at the moment...
impl<'a, BE, I> Iterator for NodeStreamer<'a, BE, I>
where
    BE: DecryptReadBackend,
    I: ReadGlobalIndex,
{
    type Item = NodeStreamItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some(node) => {
                    let path = self.path.join(node.name());
                    if self.recursive {
                        if let Some(id) = node.subtree {
                            self.path.push(node.name());
                            let be = self.be.clone();
                            let tree = match Tree::from_backend(&be, self.index, id) {
                                Ok(tree) => tree,
                                Err(err) => return Some(Err(err)),
                            };
                            let old_inner = mem::replace(&mut self.inner, tree.nodes.into_iter());
                            self.open_iterators.push(old_inner);
                        }
                    }

                    if let Some(overrides) = &self.overrides {
                        if let Match::Ignore(_) = overrides.matched(&path, false) {
                            continue;
                        }
                    }

                    return Some(Ok((path, node)));
                }
                None => match self.open_iterators.pop() {
                    Some(it) => {
                        self.inner = it;
                        _ = self.path.pop();
                    }
                    None => return None,
                },
            }
        }
    }
}

/// [`TreeStreamerOnce`] recursively visits all trees and subtrees, but each tree ID only once
///
/// # Type Parameters
///
/// * `P` - The progress indicator
#[derive(Debug)]
pub struct TreeStreamerOnce<P> {
    /// The visited tree IDs
    visited: BTreeSet<TreeId>,
    /// The queue to send tree IDs to
    queue_in: Option<Sender<(PathBuf, TreeId, usize)>>,
    /// The queue to receive trees from
    queue_out: Receiver<RusticResult<(PathBuf, Tree, usize)>>,
    /// The progress indicator
    p: P,
    /// The number of trees that are not yet finished
    counter: Vec<usize>,
    /// The number of finished trees
    finished_ids: usize,
}

impl<P: Progress> TreeStreamerOnce<P> {
    /// Creates a new `TreeStreamerOnce`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The type of the backend.
    /// * `P` - The type of the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `ids` - The IDs of the trees to visit.
    /// * `p` - The progress indicator.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::SendingCrossbeamMessageFailed`] - If sending the message fails.
    ///
    /// [`TreeErrorKind::SendingCrossbeamMessageFailed`]: crate::error::TreeErrorKind::SendingCrossbeamMessageFailed
    pub fn new<BE: DecryptReadBackend, I: ReadGlobalIndex>(
        be: &BE,
        index: &I,
        ids: Vec<TreeId>,
        p: P,
    ) -> RusticResult<Self> {
        p.set_length(ids.len() as u64);

        let (out_tx, out_rx) = bounded(constants::MAX_TREE_LOADER);
        let (in_tx, in_rx) = unbounded();

        for _ in 0..constants::MAX_TREE_LOADER {
            let be = be.clone();
            let index = index.clone();
            let in_rx = in_rx.clone();
            let out_tx = out_tx.clone();
            let _join_handle = std::thread::spawn(move || {
                for (path, id, count) in in_rx {
                    out_tx
                        .send(Tree::from_backend(&be, &index, id).map(|tree| (path, tree, count)))
                        .unwrap();
                }
            });
        }

        let counter = vec![0; ids.len()];
        let mut streamer = Self {
            visited: BTreeSet::new(),
            queue_in: Some(in_tx),
            queue_out: out_rx,
            p,
            counter,
            finished_ids: 0,
        };

        for (count, id) in ids.into_iter().enumerate() {
            if !streamer.add_pending(PathBuf::new(), id, count)? {
                streamer.p.inc(1);
                streamer.finished_ids += 1;
            }
        }

        Ok(streamer)
    }

    /// Adds a tree ID to the queue.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the tree.
    /// * `id` - The ID of the tree.
    /// * `count` - The index of the tree.
    ///
    /// # Returns
    ///
    /// Whether the tree ID was added to the queue.
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::SendingCrossbeamMessageFailed`] - If sending the message fails.
    ///
    /// [`TreeErrorKind::SendingCrossbeamMessageFailed`]: crate::error::TreeErrorKind::SendingCrossbeamMessageFailed
    fn add_pending(&mut self, path: PathBuf, id: TreeId, count: usize) -> RusticResult<bool> {
        if self.visited.insert(id) {
            self.queue_in
                .as_ref()
                .unwrap()
                .send((path, id, count))
                .map_err(TreeErrorKind::SendingCrossbeamMessageFailed)?;
            self.counter[count] += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl<P: Progress> Iterator for TreeStreamerOnce<P> {
    type Item = TreeStreamItem;

    fn next(&mut self) -> Option<Self::Item> {
        if self.counter.len() == self.finished_ids {
            drop(self.queue_in.take());
            self.p.finish();
            return None;
        }
        let (path, tree, count) = match self.queue_out.recv() {
            Ok(Ok(res)) => res,
            Err(err) => {
                return Some(Err(
                    TreeErrorKind::ReceivingCrossbreamMessageFailed(err).into()
                ))
            }
            Ok(Err(err)) => return Some(Err(err)),
        };

        for node in &tree.nodes {
            if let Some(id) = node.subtree {
                let mut path = path.clone();
                path.push(node.name());
                match self.add_pending(path, id, count) {
                    Ok(_) => {}
                    Err(err) => return Some(Err(err)),
                }
            }
        }
        self.counter[count] -= 1;
        if self.counter[count] == 0 {
            self.p.inc(1);
            self.finished_ids += 1;
        }
        Some(Ok((path, tree)))
    }
}

/// Merge trees from a list of trees
///
/// # Arguments
///
/// * `be` - The backend to read from.
/// * `trees` - The IDs of the trees to merge.
/// * `cmp` - The comparison function for the nodes.
/// * `save` - The function to save the tree.
/// * `summary` - The summary of the snapshot.
///
/// # Errors
///
// TODO!: add errors
pub(crate) fn merge_trees(
    be: &impl DecryptReadBackend,
    index: &impl ReadGlobalIndex,
    trees: &[TreeId],
    cmp: &impl Fn(&Node, &Node) -> Ordering,
    save: &impl Fn(Tree) -> RusticResult<(TreeId, u64)>,
    summary: &mut SnapshotSummary,
) -> RusticResult<TreeId> {
    // We store nodes with the index of the tree in an Binary Heap where we sort only by node name
    struct SortedNode(Node, usize);
    impl PartialEq for SortedNode {
        fn eq(&self, other: &Self) -> bool {
            self.0.name == other.0.name
        }
    }
    impl PartialOrd for SortedNode {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Eq for SortedNode {}
    impl Ord for SortedNode {
        fn cmp(&self, other: &Self) -> Ordering {
            self.0.name.cmp(&other.0.name).reverse()
        }
    }

    let mut tree_iters: Vec<_> = trees
        .iter()
        .map(|id| Tree::from_backend(be, index, *id).map(IntoIterator::into_iter))
        .collect::<RusticResult<_>>()?;

    // fill Heap with first elements from all trees
    let mut elems = BinaryHeap::new();
    for (num, iter) in tree_iters.iter_mut().enumerate() {
        if let Some(node) = iter.next() {
            elems.push(SortedNode(node, num));
        }
    }

    let mut tree = Tree::new();
    let (mut node, mut num) = match elems.pop() {
        None => {
            let (id, size) = save(tree)?;
            summary.dirs_unmodified += 1;
            summary.total_dirs_processed += 1;
            summary.total_dirsize_processed += size;
            return Ok(id);
        }
        Some(SortedNode(node, num)) => (node, num),
    };

    let mut nodes = Vec::new();
    loop {
        // push next element from tree_iters[0] (if any is left) into BinaryHeap
        if let Some(next_node) = tree_iters[num].next() {
            elems.push(SortedNode(next_node, num));
        }

        match elems.pop() {
            None => {
                // Add node to nodes list
                nodes.push(node);
                // no node left to proceed, merge nodes and quit
                tree.add(merge_nodes(be, index, nodes, cmp, save, summary)?);
                break;
            }
            Some(SortedNode(new_node, new_num)) if node.name != new_node.name => {
                // Add node to nodes list
                nodes.push(node);
                // next node has other name; merge present nodes
                tree.add(merge_nodes(be, index, nodes, cmp, save, summary)?);
                nodes = Vec::new();
                // use this node as new node
                (node, num) = (new_node, new_num);
            }
            Some(SortedNode(new_node, new_num)) => {
                // Add node to nodes list
                nodes.push(node);
                // use this node as new node
                (node, num) = (new_node, new_num);
            }
        };
    }
    let (id, size) = save(tree)?;
    if trees.contains(&id) {
        summary.dirs_unmodified += 1;
    } else {
        summary.dirs_changed += 1;
    }
    summary.total_dirs_processed += 1;
    summary.total_dirsize_processed += size;
    Ok(id)
}

/// Merge nodes from a list of nodes
///
/// # Arguments
///
/// * `be` - The backend to read from.
/// * `nodes` - The nodes to merge.
/// * `cmp` - The comparison function for the nodes.
/// * `save` - The function to save the tree.
/// * `summary` - The summary of the snapshot.
///
/// # Errors
///
// TODO: add errors
pub(crate) fn merge_nodes(
    be: &impl DecryptReadBackend,
    index: &impl ReadGlobalIndex,
    nodes: Vec<Node>,
    cmp: &impl Fn(&Node, &Node) -> Ordering,
    save: &impl Fn(Tree) -> RusticResult<(TreeId, u64)>,
    summary: &mut SnapshotSummary,
) -> RusticResult<Node> {
    let trees: Vec<_> = nodes
        .iter()
        .filter(|node| node.is_dir())
        .map(|node| node.subtree.unwrap())
        .collect();

    let mut node = nodes.into_iter().max_by(|n1, n2| cmp(n1, n2)).unwrap();

    // if this is a dir, merge with all other dirs
    if node.is_dir() {
        node.subtree = Some(merge_trees(be, index, &trees, cmp, save, summary)?);
    } else {
        summary.files_unmodified += 1;
        summary.total_files_processed += 1;
        summary.total_bytes_processed += node.meta.size;
    }
    Ok(node)
}
