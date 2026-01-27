use std::path::PathBuf;

use super::TreeId;
use crate::{
    BlobId, ErrorKind, RusticError, RusticResult,
    backend::decrypt::{DecryptFullBackend, DecryptWriteBackend},
    blob::{
        BlobType,
        packer::{PackSizer, Packer},
    },
    index::{
        ReadGlobalIndex,
        indexer::{Indexer, SharedIndexer},
    },
    repofile::{ConfigFile, Node, Tree},
};

// TODO: add documentation
#[derive(Debug, Clone, Copy)]
pub enum ModifierChange {
    /// Tree was removed
    Removed,
    /// Tree has changed
    Changed(TreeId),
    /// Tress is unchanged
    Unchanged,
}

#[derive(Debug)]
pub enum ModifierAction {
    Change(ModifierChange),
    WriteChangedTree(Tree),
    Process(TreeId),
}

#[derive(Debug)]
pub enum TreeAction {
    ProcessChangedTree(Tree),
    ProcessUnchangedTree(Tree),
}

#[derive(Debug)]
pub enum NodeAction {
    Node(Node, bool), // bool is change indicator; true: node has been changed
    Removed,
    VisitTree(TreeId, Node, bool), // bool is change indicator; true: node has been changed
    CreateTree(Node),
}

pub trait Visitor {
    fn pre_process(&self, _path: &PathBuf, _id: TreeId) -> ModifierAction {
        ModifierAction::Change(ModifierChange::Unchanged)
    }
    fn pre_process_tree(&mut self, tree: RusticResult<Tree>) -> RusticResult<TreeAction> {
        Ok(TreeAction::ProcessUnchangedTree(tree?))
    }
    fn process_node(&mut self, _path: &PathBuf, node: Node, _id: TreeId) -> NodeAction {
        if node.is_dir()
            && let Some(subtree) = node.subtree
        {
            NodeAction::VisitTree(subtree, node, false)
        } else {
            NodeAction::Node(node, false)
        }
    }
    fn post_process_tree(
        &mut self,
        _path: PathBuf,
        _tree: TreeId,
        _parent_tree: TreeId,
        modify_result: ModifierChange,
    ) -> ModifierChange {
        modify_result
    }
    fn post_process(&mut self, _path: PathBuf, _id: TreeId, _new_id: Option<TreeId>, _tree: &Tree) {
    }
}

pub struct DefaultVisitor;
impl Visitor for DefaultVisitor {}

pub struct TreeModifier<'a, BE: DecryptWriteBackend, I: ReadGlobalIndex> {
    be: &'a BE,
    index: &'a I,
    indexer: SharedIndexer<BE>,
    packer: Packer<BE>,
    dry_run: bool,
}

impl<'a, BE: DecryptFullBackend, I: ReadGlobalIndex> TreeModifier<'a, BE, I> {
    pub fn new(be: &'a BE, index: &'a I, config: &ConfigFile, dry_run: bool) -> RusticResult<Self> {
        let indexer = Indexer::new(be.clone()).into_shared();
        let pack_sizer =
            PackSizer::from_config(config, BlobType::Tree, index.total_size(BlobType::Tree));
        let packer = Packer::new(be.clone(), BlobType::Tree, indexer.clone(), pack_sizer)?;

        Ok(Self {
            be,
            index,
            indexer,
            packer,
            dry_run,
        })
    }

    pub fn modify_tree<V: Visitor>(
        &self,
        path: PathBuf,
        id: TreeId,
        visitor: &mut V,
    ) -> RusticResult<ModifierChange> {
        let mut changed = false;
        let tree = match visitor.pre_process(&path, id) {
            ModifierAction::Change(change) => return Ok(change),
            ModifierAction::WriteChangedTree(tree) => {
                changed = true;
                tree
            }
            ModifierAction::Process(id) => {
                match visitor.pre_process_tree(Tree::from_backend(self.be, self.index, id))? {
                    TreeAction::ProcessChangedTree(tree) => {
                        changed = true;
                        tree
                    }
                    TreeAction::ProcessUnchangedTree(tree) => tree,
                }
            }
        };
        let mut new_tree = Tree::new();

        for node in tree {
            let node_path = path.join(node.name());
            match visitor.process_node(&node_path, node, id) {
                NodeAction::Node(node, node_changed) => {
                    changed |= node_changed;
                    new_tree.add(node);
                }
                NodeAction::Removed => {
                    changed = true;
                }
                NodeAction::CreateTree(mut node) => {
                    changed = true;
                    node.subtree = Some(self.save_tree(&Tree::new())?);
                    new_tree.add(node);
                }
                NodeAction::VisitTree(tree, mut node, tree_changed) => {
                    changed |= tree_changed;
                    let modify_result = self.modify_tree(node_path.clone(), tree, visitor)?;
                    let modify_result =
                        visitor.post_process_tree(node_path, tree, id, modify_result);
                    match modify_result {
                        ModifierChange::Removed => {
                            changed = true;
                        }
                        ModifierChange::Unchanged => {
                            new_tree.add(node);
                        }
                        ModifierChange::Changed(tree_id) => {
                            node.subtree = Some(tree_id);
                            new_tree.add(node);
                            changed = true;
                        }
                    }
                }
            }
        }

        let new_id = if changed {
            let new_id = self.save_tree(&new_tree)?;
            (new_id != id).then_some(new_id)
        } else {
            None
        };

        visitor.post_process(path, id, new_id, &new_tree);
        Ok(new_id.map_or_else(|| ModifierChange::Unchanged, ModifierChange::Changed))
    }

    pub fn save_tree(&self, new_tree: &Tree) -> RusticResult<TreeId> {
        let (chunk, new_id) = new_tree.serialize().map_err(|err| {
            RusticError::with_source(ErrorKind::Internal, "Failed to serialize tree.", err)
                .ask_report()
        })?;

        if !self.index.has_tree(&new_id) && !self.dry_run {
            self.packer.add(chunk.into(), BlobId::from(*new_id))?;
        }
        Ok(new_id)
    }

    pub fn finalize(self) -> RusticResult<()> {
        if !self.dry_run {
            _ = self.packer.finalize()?;
            self.indexer.write().unwrap().finalize()?;
        }
        Ok(())
    }
}
