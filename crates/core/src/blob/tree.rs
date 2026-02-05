pub mod excludes;
pub mod modify;
pub mod rewrite;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    ffi::{OsStr, OsString},
    mem,
    path::{Component, Path, PathBuf, Prefix},
    str::{self, Utf8Error},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use derive_setters::Setters;
use ignore::Match;
use ignore::overrides::Override;
use serde::{Deserialize, Deserializer};
use serde_derive::Serialize;

use crate::{
    backend::{
        decrypt::DecryptReadBackend,
        node::{Metadata, Node, NodeType},
    },
    blob::{BlobType, tree::excludes::Excludes},
    crypto::hasher::hash,
    error::{ErrorKind, RusticError, RusticResult},
    impl_blobid,
    index::ReadGlobalIndex,
    progress::Progress,
    repofile::snapshotfile::SnapshotSummary,
};

/// [`TreeErrorKind`] describes the errors that can come up dealing with Trees
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum TreeErrorKind {
    /// path should not contain current or parent dir
    ContainsCurrentOrParentDirectory,
    /// `serde_json` couldn't serialize the tree: `{0:?}`
    SerializingTreeFailed(serde_json::Error),
    /// slice is not UTF-8: `{0:?}`
    PathIsNotUtf8Conform(Utf8Error),
    /// Error `{kind}` in tree streamer: `{source}`
    Channel {
        kind: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

pub(crate) type TreeResult<T> = Result<T, TreeErrorKind>;

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

    /// Serializes the tree in JSON format like restic does.
    ///
    /// # Returns
    ///
    /// A tuple of the serialized tree as `Vec<u8>` and the tree's ID, i.e. the hash of the serialized tree.
    ///
    /// # Errors
    ///
    /// * If the tree could not be serialized. This should never happen.
    pub fn serialize(&self) -> TreeResult<(Vec<u8>, TreeId)> {
        let mut chunk = serde_json::to_vec(&self).map_err(TreeErrorKind::SerializingTreeFailed)?;
        // # COMPATIBILITY
        //
        // We add a newline to be compatible with `restic` here
        chunk.push(b'\n');

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
    /// * If the tree ID is not found in the backend.
    /// * If deserialization fails.
    ///
    /// # Returns
    ///
    /// The deserialized tree.
    pub(crate) fn from_backend(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        id: TreeId,
    ) -> RusticResult<Self> {
        let data = index
            .get_tree(&id)
            .ok_or_else(|| {
                RusticError::new(
                    ErrorKind::Internal,
                    "Tree ID `{tree_id}` not found in index",
                )
                .attach_context("tree_id", id.to_string())
            })?
            .read_data(be)?;

        let tree = serde_json::from_slice(&data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to deserialize tree from JSON.",
                err,
            )
            .ask_report()
        })?;

        Ok(tree)
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
    /// * If the path is not a directory.
    /// * If the path is not found.
    /// * If the path is not UTF-8 conform.
    pub(crate) fn node_from_path(
        be: &impl DecryptReadBackend,
        index: &impl ReadGlobalIndex,
        id: TreeId,
        path: &Path,
    ) -> RusticResult<Node> {
        let mut node = Node::new_node(OsStr::new(""), NodeType::Dir, Metadata::default());
        node.subtree = Some(id);

        for p in path.components() {
            if let Some(p) = comp_to_osstr(p).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to convert Path component `{path}` to OsString.",
                    err,
                )
                .attach_context("path", path.display().to_string())
                .ask_report()
            })? {
                let id = node.subtree.ok_or_else(|| {
                    RusticError::new(ErrorKind::Internal, "Node `{node}` is not a directory.")
                        .attach_context("node", p.to_string_lossy())
                        .ask_report()
                })?;
                let tree = Self::from_backend(be, index, id)?;
                node = tree
                    .nodes
                    .into_iter()
                    .find(|node| node.name() == p)
                    .ok_or_else(|| {
                        RusticError::new(ErrorKind::Internal, "Node `{node}` not found in tree.")
                            .attach_context("node", p.to_string_lossy())
                            .ask_report()
                    })?;
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
                    let id = node.subtree.ok_or_else(|| {
                        RusticError::new(
                            ErrorKind::Internal,
                            "Subtree ID not found for node `{node}`",
                        )
                        .attach_context("node", path_comp[idx].to_string_lossy())
                        .ask_report()
                    })?;

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
            .collect::<TreeResult<_>>()
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to convert Path component `{path}` to OsString.",
                    err,
                )
                .attach_context("path", path.display().to_string())
                .ask_report()
            })?;

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
                    let id = node.subtree.ok_or_else(|| {
                        RusticError::new(
                            ErrorKind::Internal,
                            "Subtree ID not found for node `{node}`",
                        )
                        .attach_context("node", node.name().to_string_lossy())
                        .ask_report()
                    })?;

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
/// * If the component is a current or parent directory.
/// * If the component is not UTF-8 conform.
pub(crate) fn comp_to_osstr(p: Component<'_>) -> TreeResult<Option<OsString>> {
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
        _ => return Err(TreeErrorKind::ContainsCurrentOrParentDirectory),
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
#[derive(Clone, Debug, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for listing the `Nodes` of a `Tree`
pub struct TreeStreamerOptions {
    #[cfg_attr(feature = "clap", clap(flatten, next_help_heading = "Exclude options"))]
    /// exclude options
    pub excludes: Excludes,

    /// recursively list the dir
    #[cfg_attr(feature = "clap", clap(long))]
    pub recursive: bool,
}

impl Default for TreeStreamerOptions {
    fn default() -> Self {
        Self {
            excludes: Excludes::default(),
            recursive: true,
        }
    }
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
    /// * If the tree ID is not found in the backend.
    /// * If deserialization fails.
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
    /// * If the tree ID is not found in the backend.
    /// * If deserialization fails.
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
    /// * If building the streamer fails.
    /// * If reading a glob file fails.
    pub fn new_with_glob(
        be: BE,
        index: &'a I,
        node: &Node,
        opts: &TreeStreamerOptions,
    ) -> RusticResult<Self> {
        let overrides = opts.excludes.as_override()?;
        Self::new_streamer(be, index, node, Some(overrides), opts.recursive)
    }
}

// TODO: This is not parallel at the moment...
impl<BE, I> Iterator for NodeStreamer<'_, BE, I>
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
                    if self.recursive
                        && let Some(id) = node.subtree
                    {
                        self.path.push(node.name());
                        let be = self.be.clone();
                        let tree = match Tree::from_backend(&be, self.index, id) {
                            Ok(tree) => tree,
                            Err(err) => return Some(Err(err)),
                        };
                        let old_inner = mem::replace(&mut self.inner, tree.nodes.into_iter());
                        self.open_iterators.push(old_inner);
                    }

                    if let Some(overrides) = &self.overrides
                        && let Match::Ignore(_) = overrides.matched(&path, false)
                    {
                        continue;
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
pub struct TreeStreamerOnce {
    /// The visited tree IDs
    visited: BTreeSet<TreeId>,
    /// The queue to send tree IDs to
    queue_in: Option<Sender<(PathBuf, TreeId, usize)>>,
    /// The queue to receive trees from
    queue_out: Receiver<RusticResult<(PathBuf, Tree, usize)>>,
    /// The progress indicator
    p: Progress,
    /// The number of trees that are not yet finished
    counter: Vec<usize>,
    /// The number of finished trees
    finished_ids: usize,
}

impl TreeStreamerOnce {
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
    /// * If sending the message fails.
    pub fn new<BE: DecryptReadBackend, I: ReadGlobalIndex>(
        be: &BE,
        index: &I,
        ids: Vec<TreeId>,
        p: Progress,
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
            if !streamer
                .add_pending(PathBuf::new(), id, count)
                .map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to add tree ID `{tree_id}` to unbounded pending queue (`{count}`).",
                        err,
                    )
                    .attach_context("tree_id", id.to_string())
                    .attach_context("count", count.to_string())
                    .ask_report()
                })?
            {
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
    /// * If sending the message fails.
    fn add_pending(&mut self, path: PathBuf, id: TreeId, count: usize) -> TreeResult<bool> {
        if self.visited.insert(id) {
            self.queue_in
                .as_ref()
                .unwrap()
                .send((path, id, count))
                .map_err(|err| TreeErrorKind::Channel {
                    kind: "sending crossbeam message",
                    source: err.into(),
                })?;

            self.counter[count] += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Iterator for TreeStreamerOnce {
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
                return Some(Err(RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to receive tree from crossbeam channel.",
                    err,
                )
                .attach_context("finished_ids", self.finished_ids.to_string())
                .ask_report()));
            }
            Ok(Err(err)) => return Some(Err(err)),
        };

        for node in &tree.nodes {
            if let Some(id) = node.subtree {
                let mut path = path.clone();
                path.push(node.name());
                match self.add_pending(path.clone(), id, count) {
                    Ok(_) => {}
                    Err(err) => {
                        return Some(Err(err).map_err(|err| {
                            RusticError::with_source(
                                ErrorKind::Internal,
                                "Failed to add tree ID `{tree_id}` to pending queue (`{count}`).",
                                err,
                            )
                            .attach_context("path", path.display().to_string())
                            .attach_context("tree_id", id.to_string())
                            .attach_context("count", count.to_string())
                            .ask_report()
                        }));
                    }
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
        }
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
