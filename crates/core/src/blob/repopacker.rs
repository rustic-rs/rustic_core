use std::{
    num::NonZeroU32,
    sync::{Arc, RwLock},
};

use bytes::Bytes;
use chrono::Local;
use crossbeam_channel::{Receiver, Sender, bounded};
use pariter::{IteratorExt, scope};

use crate::{
    backend::{
        FileType,
        decrypt::{DecryptFullBackend, DecryptWriteBackend},
    },
    blob::{BlobId, BlobType},
    crypto::hasher::hash,
    error::{ErrorKind, RusticError, RusticResult},
    index::indexer::SharedIndexer,
    repofile::{
        configfile::ConfigFile,
        indexfile::{IndexBlob, IndexPack},
        packfile::PackId,
    },
};

use super::{
    pack_sizer::{DefaultPackSizer, FixedPackSizer, PackSizer},
    packer::{Packer, PackerStats},
};

/// The `Packer` is responsible for packing blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend type.
#[allow(missing_debug_implementations)]
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub struct RepositoryPacker<BE: DecryptWriteBackend, S> {
    /// The raw packer wrapped in an `Arc` and `RwLock`.
    // This is a hack: raw_packer and indexer are only used in the add_raw() method.
    // TODO: Refactor as actor, like the other add() methods
    packer: Arc<RwLock<Packer<BE::Key, S>>>,
    /// The shared indexer containing the backend.
    indexer: SharedIndexer,
    /// The actor to write the pack file
    file_writer: Actor,
    /// The sender to send blobs to the raw packer.
    sender: Sender<(Bytes, BlobId)>,
    /// The receiver to receive the status from the raw packer.
    finish: Receiver<RusticResult<PackerStats>>,
}

impl<BE: DecryptWriteBackend> RepositoryPacker<BE, DefaultPackSizer> {
    #[allow(clippy::unnecessary_wraps)]
    pub fn new_with_default_sizer(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer,
        config: &ConfigFile,
        total_size: u64,
    ) -> RusticResult<Self> {
        let pack_sizer = DefaultPackSizer::from_config(config, BlobType::Data, total_size);
        Self::new(be, blob_type, indexer, pack_sizer)
    }
}

impl<BE: DecryptWriteBackend, S: PackSizer + Send + Sync + 'static> RepositoryPacker<BE, S> {
    /// Creates a new `Packer`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend type.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to write to.
    /// * `blob_type` - The blob type.
    /// * `indexer` - The indexer to write to.
    /// * `config` - The config file.
    /// * `total_size` - The total size of the pack file.
    ///
    /// # Errors
    ///
    /// * If sending the message to the raw packer fails.
    /// * If converting the data length to u64 fails
    #[allow(clippy::unnecessary_wraps)]
    pub fn new(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer,
        pack_sizer: S,
    ) -> RusticResult<Self> {
        let packer = Arc::new(RwLock::new(Packer::new(*be.key(), pack_sizer, blob_type)));

        let file_writer = Actor::new(
            FileWriterHandle {
                be: be.clone(),
                indexer: indexer.clone(),
                cacheable: blob_type.is_cacheable(),
            },
            1,
            1,
        );

        let (tx, rx) = bounded(0);
        let (finish_tx, finish_rx) = bounded::<RusticResult<PackerStats>>(0);
        let repository_packer = Self {
            packer: packer.clone(),
            indexer: indexer.clone(),
            file_writer: file_writer.clone(),
            sender: tx,
            finish: finish_rx,
        };

        let _join_handle = std::thread::spawn(move || {
            scope(|scope| {
                let status = rx
                    .into_iter()
                    .readahead_scoped(scope)
                    // early check if id is already contained and reserve, if not
                    .filter(|(_, id)| indexer.reserve(id))
                    .parallel_map_scoped(scope, |(data, id): (Bytes, BlobId)| {
                        let (data, data_len, uncompressed_length) = be.process_data(&data)?;
                        Ok((data, id, u64::from(data_len), uncompressed_length))
                    })
                    .readahead_scoped(scope)
                    .try_for_each(|item: RusticResult<_>| -> RusticResult<()> {
                        let (data, id, data_len, ul) = item?;
                        let res = {
                            let mut raw_packer = packer.write().unwrap();
                            raw_packer.add(&data, &id, data_len, ul)?;
                            raw_packer.save_if_needed()?
                        };
                        if let Some((file, index)) = res {
                            file_writer.send((file, index))?;
                        }

                        Ok(())
                    })
                    .and_then(|()| {
                        let (res, stats) = packer.write().unwrap().finalize()?;
                        if let Some((file, index)) = res {
                            file_writer.send((file, index))?;
                        }
                        Ok(stats)
                    });
                _ = finish_tx.send(status);
            })
            .unwrap();
        });

        Ok(repository_packer)
    }

    /// Adds the blob to the packfile
    ///
    /// # Arguments
    ///
    /// * `data` - The blob data
    /// * `id` - The blob id
    ///
    /// # Errors
    ///
    /// * If sending the message to the raw packer fails.
    pub fn add(&self, data: Bytes, id: BlobId) -> RusticResult<()> {
        // compute size limit based on total size and size bounds
        self.sender.send((data, id)).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to add blob `{id}` to packfile.",
                err,
            )
            .attach_context("id", id.to_string())
            .ask_report()
        })
    }

    /// Adds the already encrypted (and maybe compressed) blob to the packfile
    ///
    /// # Arguments
    ///
    /// * `data` - The blob data
    /// * `id` - The blob id
    /// * `data_len` - The length of the blob data
    /// * `uncompressed_length` - The length of the blob data before compression
    /// * `size_limit` - The size limit for the pack file
    ///
    /// # Errors
    ///
    /// * If the blob is already present in the index
    /// * If sending the message to the raw packer fails.
    fn add_raw(
        &self,
        data: &[u8],
        id: &BlobId,
        data_len: u64,
        uncompressed_length: Option<NonZeroU32>,
    ) -> RusticResult<()> {
        // only add if this blob is not present
        if self.indexer.reserve(id) {
            let mut raw_packer = self.packer.write().unwrap();
            raw_packer.add(data, id, data_len, uncompressed_length)?;

            if let Some((file, index)) = raw_packer.save_if_needed()? {
                self.file_writer.send((file, index)).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to send packfile to file writer.",
                        err,
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Finalizes the packer and does cleanup
    ///
    /// # Panics
    ///
    /// * If the channel could not be dropped
    pub fn finalize(self) -> RusticResult<PackerStats> {
        // cancel channel
        drop(self.sender);
        // wait for items in channel to be processed
        let res = self
            .finish
            .recv()
            .expect("Should be able to receive from channel to finalize packer.");
        self.file_writer.finalize()?;
        res
    }
}

// TODO: add documentation
/// # Type Parameters
///
/// * `BE` - The backend type.
#[derive(Clone)]
pub(crate) struct FileWriterHandle<BE: DecryptWriteBackend> {
    /// The backend to write to.
    be: BE,
    /// The shared indexer containing the backend.
    indexer: SharedIndexer,
    /// Whether the file is cacheable.
    cacheable: bool,
}

impl<BE: DecryptWriteBackend> FileWriterHandle<BE> {
    // TODO: add documentation
    fn process(&self, load: (Bytes, PackId, IndexPack)) -> RusticResult<IndexPack> {
        let (file, id, mut index) = load;
        index.id = id;
        self.be
            .write_bytes(FileType::Pack, &id, self.cacheable, file)?;
        index.time = Some(Local::now());
        Ok(index)
    }

    fn index(&self, index: IndexPack) -> RusticResult<()> {
        self.indexer
            .add_and_check_save(index, false, |file| self.be.save_file_no_id(file))
    }
}

// TODO: add documentation
#[derive(Clone)]
pub(crate) struct Actor {
    /// The sender to send blobs to the raw packer.
    sender: Sender<(Bytes, IndexPack)>,
    /// The receiver to receive the status from the raw packer.
    finish: Receiver<RusticResult<()>>,
}

impl Actor {
    /// Creates a new `Actor`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend type.
    ///
    /// # Arguments
    ///
    /// * `fwh` - The file writer handle.
    /// * `queue_len` - The length of the queue.
    /// * `par` - The number of parallel threads.
    fn new<BE: DecryptWriteBackend>(
        fwh: FileWriterHandle<BE>,
        queue_len: usize,
        _par: usize,
    ) -> Self {
        let (tx, rx) = bounded(queue_len);
        let (finish_tx, finish_rx) = bounded::<RusticResult<()>>(0);

        let _join_handle = std::thread::spawn(move || {
            scope(|scope| {
                let status = rx
                    .into_iter()
                    .readahead_scoped(scope)
                    .map(|(file, index): (Bytes, IndexPack)| {
                        let id = hash(&file);
                        (file, PackId::from(id), index)
                    })
                    .readahead_scoped(scope)
                    .map(|load| fwh.process(load))
                    .readahead_scoped(scope)
                    .try_for_each(|index| fwh.index(index?));
                _ = finish_tx.send(status);
            })
            .unwrap();
        });

        Self {
            sender: tx,
            finish: finish_rx,
        }
    }

    /// Sends the given data to the actor.
    ///
    /// # Arguments
    ///
    /// * `load` - The data to send.
    ///
    /// # Errors
    ///
    /// If sending the message to the actor fails.
    fn send(&self, load: (Bytes, IndexPack)) -> RusticResult<()> {
        self.sender.send(load).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to send packfile to file writer.",
                err,
            )
        })?;
        Ok(())
    }

    /// Finalizes the actor and does cleanup
    ///
    /// # Panics
    ///
    /// * If the receiver is not present
    fn finalize(self) -> RusticResult<()> {
        // cancel channel
        drop(self.sender);
        // wait for items in channel to be processed
        self.finish.recv().unwrap()
    }
}

/// The `Repacker` is responsible for repacking blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend to read from.
#[allow(missing_debug_implementations)]
pub struct Repacker<BE>
where
    BE: DecryptFullBackend,
{
    /// The backend to read from.
    be: BE,
    /// The packer to write to.
    packer: RepositoryPacker<BE, FixedPackSizer>,
    /// The size limit of the pack file.
    size_limit: u32,
}

impl<BE: DecryptFullBackend> Repacker<BE> {
    /// Creates a new `Repacker`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend to read from.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from.
    /// * `blob_type` - The blob type.
    /// * `indexer` - The indexer to write to.
    /// * `config` - The config file.
    /// * `total_size` - The total size of the pack file.
    ///
    /// # Errors
    ///
    /// * If the Packer could not be created
    pub fn new(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer,
        config: &ConfigFile,
        total_size: u64,
    ) -> RusticResult<Self> {
        let size_limit = DefaultPackSizer::from_config(config, blob_type, total_size).pack_size();
        let repo_sizer = FixedPackSizer(size_limit);
        let packer = RepositoryPacker::new(be.clone(), blob_type, indexer, repo_sizer)?;
        Ok(Self {
            be,
            packer,
            size_limit,
        })
    }

    /// Adds the blob to the packfile without any check
    ///
    /// # Arguments
    ///
    /// * `pack_id` - The pack id
    /// * `blob` - The blob to add
    ///
    /// # Errors
    ///
    /// * If the blob could not be added
    /// * If reading the blob from the backend fails
    pub fn add_fast(&self, pack_id: &PackId, blob: &IndexBlob) -> RusticResult<()> {
        let data = self.be.read_partial(
            FileType::Pack,
            pack_id,
            blob.tpe.is_cacheable(),
            blob.offset,
            blob.length,
        )?;

        self.packer
            .add_raw(&data, &blob.id, 0, blob.uncompressed_length)
            .map_err(|err| {
                err.overwrite_kind(ErrorKind::Internal)
                    .prepend_guidance_line(
                        "Failed to fast-add (unchecked) blob `{blob_id}` to packfile.",
                    )
                    .attach_context("blob_id", blob.id.to_string())
            })?;

        Ok(())
    }

    /// Adds the blob to the packfile
    ///
    /// # Arguments
    ///
    /// * `pack_id` - The pack id
    /// * `blob` - The blob to add
    ///
    /// # Errors
    ///
    /// * If the blob could not be added
    /// * If reading the blob from the backend fails
    pub fn add(&self, pack_id: &PackId, blob: &IndexBlob) -> RusticResult<()> {
        let data = self.be.read_encrypted_partial(
            FileType::Pack,
            pack_id,
            blob.tpe.is_cacheable(),
            blob.offset,
            blob.length,
            blob.uncompressed_length,
        )?;

        self.packer.add(data, blob.id)?;

        Ok(())
    }

    /// Finalizes the repacker and returns the stats
    pub fn finalize(self) -> RusticResult<PackerStats> {
        self.packer.finalize()
    }
}
