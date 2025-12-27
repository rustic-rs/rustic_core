//! `repair snapshots` subcommand
use derive_setters::Setters;
use log::{info, warn};

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::{
    backend::{decrypt::DecryptWriteBackend, node::NodeType},
    blob::tree::{
        Tree, TreeId,
        modify::{ModifierAction, ModifierChange, NodeAction, TreeAction, TreeModifier, Visitor},
    },
    error::{ErrorKind, RusticError, RusticResult},
    index::ReadGlobalIndex,
    progress::ProgressBars,
    repofile::{Node, SnapshotFile, StringList, snapshotfile::SnapshotId},
    repository::{IndexedFull, IndexedTree, Repository},
};

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Debug, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `repair snapshots` command
pub struct RepairSnapshotsOptions {
    /// Also remove defect snapshots
    ///
    /// # Warning
    ///
    /// * This can result in data loss!
    #[cfg_attr(feature = "clap", clap(long))]
    pub delete: bool,

    /// Append this suffix to repaired directory or file name
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "SUFFIX", default_value = ".repaired")
    )]
    pub suffix: String,

    /// Tag list to set on repaired snapshots (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "TAG[,TAG,..]", default_value = "repaired")
    )]
    pub tag: Vec<StringList>,
}

impl Default for RepairSnapshotsOptions {
    fn default() -> Self {
        Self {
            delete: true,
            suffix: ".repaired".to_string(),
            tag: vec![StringList(BTreeSet::from(["repaired".to_string()]))],
        }
    }
}

pub(crate) struct RepairState<'a, I: ReadGlobalIndex> {
    opts: &'a RepairSnapshotsOptions,
    index: &'a I,
    changed: BTreeMap<TreeId, TreeId>,
    unchanged: BTreeSet<TreeId>,
    delete: Vec<SnapshotId>,
}

impl<'a, I: ReadGlobalIndex> RepairState<'a, I> {
    fn new(opts: &'a RepairSnapshotsOptions, index: &'a I) -> Self {
        Self {
            opts,
            index,
            changed: BTreeMap::new(),
            unchanged: BTreeSet::new(),
            delete: Vec::new(),
        }
    }
}

impl<I: ReadGlobalIndex> Visitor for RepairState<'_, I> {
    fn pre_process(&self, _path: &PathBuf, id: TreeId) -> ModifierAction {
        if self.unchanged.contains(&id) {
            ModifierAction::Change(ModifierChange::Unchanged)
        } else if let Some(r) = self.changed.get(&id) {
            ModifierAction::Change(ModifierChange::Changed(*r))
        } else {
            ModifierAction::Process(id)
        }
    }
    fn pre_process_tree(&mut self, tree: RusticResult<Tree>) -> RusticResult<TreeAction> {
        Ok(tree.map_or_else(
            |err| {
                warn!("{}", err.display_log()); // TODO: id in error message
                TreeAction::ProcessChangedTree(Tree::new())
            },
            TreeAction::ProcessUnchangedTree,
        ))
    }

    fn process_node(&mut self, _path: &PathBuf, mut node: Node, _id: TreeId) -> NodeAction {
        match node.node_type {
            NodeType::File => {
                let mut file_changed = false;
                let mut new_content = Vec::new();
                let mut new_size = 0;
                for blob in node.content.take().unwrap() {
                    self.index.get_data(&blob).map_or_else(
                        || {
                            file_changed = true;
                        },
                        |ie| {
                            new_content.push(blob);
                            new_size += u64::from(ie.data_length());
                        },
                    );
                }
                if file_changed {
                    warn!("file {}: contents are missing", node.name);
                    node.name += &self.opts.suffix;
                } else if new_size != node.meta.size {
                    info!("file {}: corrected file size", node.name);
                }
                node.content = Some(new_content);
                node.meta.size = new_size;
                if file_changed {
                    NodeAction::ChangedNode(node)
                } else {
                    NodeAction::UnchangedNode(node)
                }
            }
            NodeType::Dir => {
                if let Some(subtree) = node.subtree {
                    NodeAction::VisitTree(subtree, node)
                } else {
                    NodeAction::CreateTree(node)
                }
            }
            _ => NodeAction::UnchangedNode(node), // Other types: no check needed
        }
    }
    fn post_process(
        &mut self,
        _path: PathBuf,
        id: TreeId,
        changed: bool,
        new_id: Option<TreeId>,
        _tree: &Tree,
    ) {
        if changed {
            if let Some(new_id) = new_id {
                _ = self.changed.insert(id, new_id);
            }
        } else {
            _ = self.unchanged.insert(id);
        }
    }
}

/// Runs the `repair snapshots` command
///
/// # Type Parameters
///
/// * `P` - The progress bar type
/// * `S` - The type of the indexed tree.
///
/// # Arguments
///
/// * `repo` - The repository to repair
/// * `opts` - The repair options to use
/// * `snapshots` - The snapshots to repair
/// * `dry_run` - Whether to actually modify the repository or just print what would be done
pub(crate) fn repair_snapshots<P: ProgressBars, S: IndexedFull>(
    repo: &Repository<P, S>,
    opts: &RepairSnapshotsOptions,
    snapshots: Vec<SnapshotFile>,
    dry_run: bool,
) -> RusticResult<()> {
    let be = repo.dbe();
    let config_file = repo.config();

    if opts.delete && config_file.append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Removing snapshots is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
    }

    let mut state = RepairState::new(opts, repo.index());
    let modifier = TreeModifier::new(be, repo.index(), config_file, dry_run)?;

    for mut snap in snapshots {
        let snap_id = snap.id;
        info!("processing snapshot {snap_id}");
        match modifier.modify_tree(PathBuf::new(), snap.tree, &mut state)? {
            ModifierChange::Unchanged => {
                info!("snapshot {snap_id} is ok.");
            }
            ModifierChange::Removed => {
                warn!("snapshot {snap_id}: root tree is damaged -> marking for deletion!");
                state.delete.push(snap_id);
            }
            ModifierChange::Changed(id) => {
                // change snapshot tree
                if snap.original.is_none() {
                    snap.original = Some(snap.id);
                }
                _ = snap.set_tags(opts.tag.clone());
                snap.tree = id;
                if dry_run {
                    info!("would have modified snapshot {snap_id}.");
                } else {
                    let new_id = be.save_file(&snap)?;
                    info!("saved modified snapshot as {new_id}.");
                }
                state.delete.push(snap_id);
            }
        }
    }

    if opts.delete {
        if dry_run {
            info!("would have removed {} snapshots.", state.delete.len());
        } else {
            be.delete_list(
                true,
                state.delete.iter(),
                repo.pb.progress_counter("remove defect snapshots"),
            )?;
        }
    }

    Ok(())
}
