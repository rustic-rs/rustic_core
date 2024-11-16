use std::io::Read;

use rustic_cdc::Rabin64;

use crate::{
    archiver::{
        parent::{ItemWithParent, ParentResult},
        tree::TreeType,
        tree_archiver::TreeItem,
    },
    backend::{
        decrypt::DecryptWriteBackend,
        node::{Node, NodeType},
        ReadSourceOpen,
    },
    blob::{
        packer::{Packer, PackerStats},
        BlobId, BlobType, DataId,
    },
    chunker::ChunkIter,
    crypto::hasher::hash,
    error::{ErrorKind, RusticError, RusticResult},
    index::{indexer::SharedIndexer, ReadGlobalIndex},
    progress::Progress,
    repofile::configfile::ConfigFile,
};

/// The `FileArchiver` is responsible for archiving files.
/// It will read the file, chunk it, and write the chunks to the backend.
///
/// # Type Parameters
///
/// * `BE` - The backend type.
/// * `I` - The index to read from.
#[derive(Clone)]
pub(crate) struct FileArchiver<'a, BE: DecryptWriteBackend, I: ReadGlobalIndex> {
    index: &'a I,
    data_packer: Packer<BE>,
    rabin: Rabin64,
}

impl<'a, BE: DecryptWriteBackend, I: ReadGlobalIndex> FileArchiver<'a, BE, I> {
    /// Creates a new `FileArchiver`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend type.
    /// * `I` - The index to read from.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to write to.
    /// * `index` - The index to read from.
    /// * `indexer` - The indexer to write to.
    /// * `config` - The config file.
    ///
    /// # Errors
    ///
    /// * If sending the message to the raw packer fails.
    /// * If converting the data length to u64 fails
    pub(crate) fn new(
        be: BE,
        index: &'a I,
        indexer: SharedIndexer<BE>,
        config: &ConfigFile,
    ) -> RusticResult<Self> {
        let poly = config.poly()?;

        let data_packer = Packer::new(
            be,
            BlobType::Data,
            indexer,
            config,
            index.total_size(BlobType::Data),
        )?;

        let rabin = Rabin64::new_with_polynom(6, &poly);

        Ok(Self {
            index,
            data_packer,
            rabin,
        })
    }

    /// Processes the given item.
    ///
    /// # Type Parameters
    ///
    /// * `O` - The type of the tree item.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to process.
    /// * `p` - The progress tracker.
    ///
    /// # Errors
    ///
    /// * If the item could not be unpacked.
    ///
    /// # Returns
    ///
    /// The processed item.
    pub(crate) fn process<O: ReadSourceOpen>(
        &self,
        item: ItemWithParent<Option<O>>,
        p: &impl Progress,
    ) -> RusticResult<TreeItem> {
        Ok(match item {
            TreeType::NewTree(item) => TreeType::NewTree(item),
            TreeType::EndTree => TreeType::EndTree,
            TreeType::Other((path, node, (open, parent))) => {
                let (node, filesize) = if matches!(parent, ParentResult::Matched(())) {
                    let size = node.meta.size;
                    p.inc(size);
                    (node, size)
                } else if node.node_type == NodeType::File {
                    let r = open
                        .ok_or_else(
                            || RusticError::new(
                                ErrorKind::Internal,
                                "Failed to unpack tree type optional at `{path}`. Option should contain a value, but contained `None`.",
                            )
                            .attach_context("path", path.display().to_string())
                            .ask_report(),
                        )?
                        .open()
                        .map_err(|err| {
                            err
                            .overwrite_kind(ErrorKind::InputOutput)
                            .prepend_guidance_line("Failed to open ReadSourceOpen at `{path}`")
                            .attach_context("path", path.display().to_string())
                        })?;

                    self.backup_reader(r, node, p)?
                } else {
                    (node, 0)
                };
                TreeType::Other((path, node, (parent, filesize)))
            }
        })
    }

    // TODO: add documentation!
    fn backup_reader(
        &self,
        r: impl Read + Send + 'static,
        node: Node,
        p: &impl Progress,
    ) -> RusticResult<(Node, u64)> {
        let chunks: Vec<_> = ChunkIter::new(
            r,
            usize::try_from(node.meta.size).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to convert node size `{size}` to usize",
                    err,
                )
                .attach_context("size", node.meta.size.to_string())
            })?,
            self.rabin.clone(),
        )
        .map(|chunk| {
            let chunk = chunk?;
            let id = hash(&chunk);
            let size = chunk.len() as u64;

            if !self.index.has_data(&DataId::from(id)) {
                self.data_packer.add(chunk.into(), BlobId::from(id))?;
            }
            p.inc(size);
            Ok((DataId::from(id), size))
        })
        .collect::<RusticResult<_>>()?;

        let filesize = chunks.iter().map(|x| x.1).sum();
        let content = chunks.into_iter().map(|x| x.0).collect();

        let mut node = node;
        node.content = Some(content);
        Ok((node, filesize))
    }

    /// Finalizes the archiver.
    ///
    /// # Returns
    ///
    /// The statistics of the archiver.
    ///
    /// # Panics
    ///
    /// * If the channel could not be dropped
    pub(crate) fn finalize(self) -> RusticResult<PackerStats> {
        self.data_packer.finalize()
    }
}
