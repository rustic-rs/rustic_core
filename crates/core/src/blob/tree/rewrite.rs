use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    ops::AddAssign,
    path::PathBuf,
};

use derive_more::Add;
use ignore::{Match, overrides::Override};

use crate::{
    RusticResult, TreeId,
    backend::decrypt::{DecryptFullBackend, DecryptWriteBackend},
    blob::tree::{
        excludes::Excludes,
        modify::{
            DefaultVisitor, ModifierAction, ModifierChange, NodeAction, TreeModifier, Visitor,
        },
    },
    index::ReadGlobalIndex,
    repofile::{ConfigFile, Node, Tree},
};

#[derive(Debug)]
pub struct RewriteVisitor<'a, I: ReadGlobalIndex> {
    index: &'a I,
    changed: BTreeMap<(PathBuf, TreeId), TreeId>,
    unchanged: BTreeSet<(PathBuf, TreeId)>,
    overrides: Override,
    summary: BTreeMap<TreeId, Summary>,
}

impl<'a, I: ReadGlobalIndex> RewriteVisitor<'a, I> {
    pub fn new(index: &'a I, excludes: &Excludes) -> RusticResult<Self> {
        Ok(Self {
            index,
            changed: BTreeMap::new(),
            unchanged: BTreeSet::new(),
            overrides: excludes.as_override()?,
            summary: BTreeMap::new(),
        })
    }
}

impl<I: ReadGlobalIndex> Visitor for RewriteVisitor<'_, I> {
    fn pre_process(&self, path: &PathBuf, id: TreeId) -> ModifierAction {
        if let Match::Ignore(_) = self.overrides.matched(path, false) {
            ModifierAction::Change(ModifierChange::Removed)
        } else if self.unchanged.contains(&(path.clone(), id)) {
            ModifierAction::Change(ModifierChange::Unchanged)
        } else if let Some(r) = self.changed.get(&(path.clone(), id)) {
            ModifierAction::Change(ModifierChange::Changed(*r))
        } else {
            ModifierAction::Process(id)
        }
    }

    fn process_node(&mut self, path: &PathBuf, node: Node, id: TreeId) -> NodeAction {
        self.summary.entry(id).or_default().update(&node);
        if let Match::Ignore(_) = self.overrides.matched(path, node.is_dir()) {
            NodeAction::Removed
        } else {
            DefaultVisitor.process_node(path, node, id)
        }
    }
    fn post_process_tree(
        &mut self,
        path: PathBuf,
        tree: TreeId,
        parent_tree: TreeId,
        modify_result: ModifierChange,
    ) -> ModifierChange {
        let summary = self
            .summary
            .get(&tree)
            .map_or_else(Default::default, Clone::clone);
        self.summary
            .entry(parent_tree)
            .or_default()
            .add_assign(summary);

        DefaultVisitor.post_process_tree(path, tree, parent_tree, modify_result)
    }

    fn post_process(
        &mut self,
        path: PathBuf,
        id: TreeId,
        changed: bool,
        new_id: Option<TreeId>,
        tree: &Tree,
    ) {
        let mut summary = Summary::default();
        summary.dirs += 1;
        for node in &tree.nodes {
            if node.is_dir()
                && let Some(subtree) = node.subtree
            {
                let tree_summary = self
                    .summary
                    .get(&subtree)
                    .map_or_else(Default::default, Clone::clone);
                summary += tree_summary;
            } else {
                summary.update(node);
            }
        }
        let _ = self.summary.insert(new_id.unwrap_or(id), summary);
        if changed {
            if let Some(new_id) = new_id {
                _ = self.changed.insert((path, id), new_id);
            }
        } else {
            _ = self.unchanged.insert((path, id));
        }
    }
}

pub struct Rewriter<'a, BE: DecryptWriteBackend, I: ReadGlobalIndex> {
    modifier: TreeModifier<'a, BE, I>,
    visitor: RewriteVisitor<'a, I>,
}

impl<'a, BE: DecryptFullBackend, I: ReadGlobalIndex> Rewriter<'a, BE, I> {
    pub fn new(
        be: &'a BE,
        index: &'a I,
        config: &ConfigFile,
        dry_run: bool,
        excludes: &Excludes,
    ) -> RusticResult<Self> {
        let modifier = TreeModifier::new(be, index, config, dry_run)?;
        let visitor = RewriteVisitor::new(index, excludes)?;
        Ok(Self { modifier, visitor })
    }

    pub fn rewrite_tree(&mut self, path: PathBuf, id: TreeId) -> RusticResult<ModifierChange> {
        self.modifier.modify_tree(path, id, &mut self.visitor)
    }

    pub fn summary(&self, tree_id: &TreeId) -> Option<&Summary> {
        self.visitor.summary.get(tree_id)
    }

    pub fn finalize(self) -> RusticResult<()> {
        self.modifier.finalize()
    }
}

/// Summary
#[derive(Debug, Default, Clone, Copy, Add)]
pub struct Summary {
    pub files: u64,
    pub size: u64,
    pub dirs: u64,
}

impl AddAssign for Summary {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Summary {
    /// Update the summary with the node
    ///
    /// # Arguments
    ///
    /// * `node` - the node to update the summary with
    pub fn update(&mut self, node: &Node) {
        if node.is_dir() {
            self.dirs += 1;
        }
        if node.is_file() {
            self.files += 1;
            self.size += node.meta.size;
        }
    }

    pub fn from_node(node: &Node) -> Self {
        let mut summary = Self::default();
        summary.update(node);
        summary
    }
}
