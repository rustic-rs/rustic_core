//! `repair snapshots` subcommand
use derive_setters::Setters;
use log::{info, warn};

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::{
    backend::{decrypt::DecryptWriteBackend, node::NodeType},
    blob::tree::modify::{
        ModifierAction, ModifierChange, NodeAction, TreeAction, TreeModifier, Visitor,
    },
    blob::tree::{Tree, TreeId},
    error::{ErrorKind, RusticError, RusticResult},
    index::ReadGlobalIndex,
    repofile::{Node, SnapshotFile, StringList, snapshotfile::SnapshotId},
    repository::{IndexedFull, Repository},
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
                if let Some(content) = node.content.take() {
                    for blob in content {
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
                } else {
                    file_changed = true;
                }
                if file_changed {
                    warn!("file {}: contents are missing", node.name);
                    node.name += &self.opts.suffix;
                } else if new_size != node.meta.size {
                    info!("file {}: corrected file size", node.name);
                }
                node.content = Some(new_content);
                node.meta.size = new_size;
                NodeAction::Node(node, file_changed)
            }
            NodeType::Dir => {
                if let Some(subtree) = node.subtree {
                    NodeAction::VisitTree(subtree, node, false)
                } else {
                    NodeAction::CreateTree(node)
                }
            }
            _ => NodeAction::Node(node, false), // Other types: no check needed
        }
    }
    fn post_process(&mut self, _path: PathBuf, id: TreeId, new_id: Option<TreeId>, _tree: &Tree) {
        if let Some(new_id) = new_id {
            _ = self.changed.insert(id, new_id);
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
pub(crate) fn repair_snapshots<S: IndexedFull>(
    repo: &Repository<S>,
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
    let mut repaired_snapshots = Vec::new();

    for mut snap in snapshots {
        let snap_id = snap.id;
        info!("processing snapshot {snap_id}");
        // match repair_tree(
        //     repo.dbe(),
        //     opts,
        //     repo.index(),
        //    &packer,
        //     Some(snap.tree),
        //     &mut state,
        //     dry_run,
        // )? {
        //     (Changed::None, _) => {
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
                    repaired_snapshots.push(snap);
                }
                state.delete.push(snap_id);
            }
        }
    }
    modifier.finalize()?;

    for snap in repaired_snapshots {
        let new_id = be.save_file(&snap)?;
        info!("saved modified snapshot as {new_id}.");
    }

    if opts.delete {
        if dry_run {
            info!("would have removed {} snapshots.", state.delete.len());
        } else {
            be.delete_list(
                true,
                state.delete.iter(),
                repo.progress_counter("remove defect snapshots"),
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use rstest::rstest;

    use crate::{
        BackupOptions, CheckOptions, ConfigOptions, Credentials, FileType, IndexedFullStatus,
        KeyOptions, ParentOptions, RepairIndexOptions, Repository, RepositoryBackends,
        RepositoryOptions, WriteBackend,
        commands::check::CheckError,
        index::{
            GlobalIndex,
            binarysorted::{IndexCollector, IndexType},
        },
        repofile::{MasterKey, SnapshotFile, SnapshotId},
        test_support::{MemoryBackend, MemorySource},
    };

    use super::*;

    #[test]
    fn test_process_node_file_without_content_does_not_panic() {
        let opts = RepairSnapshotsOptions::default();
        let index = GlobalIndex::new_from_index(IndexCollector::new(IndexType::Full).into_index());
        let mut state = RepairState::new(&opts, &index);

        let node = Node {
            name: "corrupt_file".to_string(),
            node_type: NodeType::File,
            content: None,
            ..Default::default()
        };

        let action = state.process_node(&PathBuf::new(), node, TreeId::default());
        match action {
            NodeAction::Node(repaired_node, changed) => {
                assert!(changed);
                assert_eq!(repaired_node.name, "corrupt_file.repaired");
                assert_eq!(repaired_node.content, Some(Vec::new()));
                assert_eq!(repaired_node.meta.size, 0);
            }
            _ => panic!("expected NodeAction::Node"),
        }
    }

    fn repository_with_missing_data() -> anyhow::Result<(
        Arc<MemoryBackend>,
        Repository<IndexedFullStatus>,
        SnapshotFile,
    )> {
        let backend = Arc::new(MemoryBackend::default());
        let backends = RepositoryBackends::new(backend.clone(), None);
        let repo = Repository::new(&RepositoryOptions::default().no_cache(true), &backends)?
            .init(
                &Credentials::Masterkey(MasterKey::new()),
                &KeyOptions::default(),
                &ConfigOptions::default(),
            )?
            .to_indexed_ids()?;
        let source = MemorySource {
            files: vec![("queue/00/message", b"message")],
        };
        let options = BackupOptions::default()
            .as_path(PathBuf::from("backup"))
            .parent_opts(ParentOptions::default().force(true));
        let snapshot = repo.archive(
            &options,
            &source,
            SnapshotFile::default(),
            &[PathBuf::from("source")],
        )?;
        let repo = repo.to_indexed()?;
        repo.check(CheckOptions::default().read_data(true))?
            .is_ok()?;
        let node = repo.node_from_path(snapshot.tree, Path::new("backup/queue/00/message"))?;
        let data = node.content.unwrap()[0];
        let pack = repo.get_index_entry(&data)?.pack;
        backend.remove(FileType::Pack, &pack, false)?;
        repo.repair_index(&RepairIndexOptions::default(), false)?;
        let repo = repo.to_indexed()?;
        let results = repo.check(CheckOptions::default())?;
        assert!(results.0.iter().any(|(_, error)| {
            matches!(error, CheckError::FileBlobNotInIndex { blob_id, .. } if *blob_id == data)
        }));
        Ok((backend, repo, snapshot))
    }

    #[rstest]
    #[case::pack_write(FileType::Pack)]
    #[case::index_write(FileType::Index)]
    fn repair_publishes_no_snapshot_when_tree_flush_fails(
        #[case] failure: FileType,
    ) -> anyhow::Result<()> {
        let (backend, repo, snapshot) = repository_with_missing_data()?;
        let original_ids = repo.list::<SnapshotId>()?.collect::<BTreeSet<_>>();
        assert_eq!(original_ids, BTreeSet::from([snapshot.id]));
        backend.fail_writes(failure);

        let error = repo
            .repair_snapshots(&RepairSnapshotsOptions::default(), vec![snapshot], false)
            .expect_err("the injected tree flush failure must propagate");
        assert!(backend.failed_writes() > 0);
        assert!(
            error.display_log().contains("Injected test write failure"),
            "unexpected repair failure: {}",
            error.display_log(),
        );
        let remaining_ids = repo.list::<SnapshotId>()?.collect::<BTreeSet<_>>();
        assert_eq!(
            remaining_ids, original_ids,
            "repair published a snapshot before its tree pack and index were successfully flushed",
        );
        Ok(())
    }

    #[test]
    fn repair_publishes_readable_snapshot_after_successful_flush() -> anyhow::Result<()> {
        let (_, repo, snapshot) = repository_with_missing_data()?;
        let original_id = snapshot.id;
        repo.repair_snapshots(&RepairSnapshotsOptions::default(), vec![snapshot], false)?;
        let snapshots = repo.get_all_snapshots()?;
        assert_eq!(snapshots.len(), 1);
        assert_ne!(snapshots[0].id, original_id);
        let repo = repo.to_indexed()?;
        repo.check(CheckOptions::default().read_data(true))?
            .is_ok()?;
        let node = repo.node_from_path(
            snapshots[0].tree,
            Path::new("backup/queue/00/message.repaired"),
        )?;
        assert_eq!(node.meta.size, 0);
        assert_eq!(node.content, Some(Vec::new()));
        Ok(())
    }
}
