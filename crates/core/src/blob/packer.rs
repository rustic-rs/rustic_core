use std::{
    num::NonZeroU32,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime},
};

use bytes::{Bytes, BytesMut};
use crossbeam_channel::{Receiver, Sender, bounded};
use integer_sqrt::IntegerSquareRoot;
use jiff::Timestamp;
use log::warn;
use pariter::{IteratorExt, scope};

use crate::{
    Progress,
    backend::{
        FileType,
        decrypt::{DecryptFullBackend, DecryptWriteBackend},
    },
    blob::{BlobId, BlobLocations, BlobType},
    crypto::{CryptoKey, hasher::hash},
    error::{ErrorKind, RusticError, RusticResult},
    index::{IndexEntry, indexer::SharedIndexer},
    repofile::{
        configfile::ConfigFile,
        indexfile::{IndexBlob, IndexPack},
        packfile::{PackHeaderLength, PackHeaderRef, PackId},
        snapshotfile::SnapshotSummary,
    },
};

/// [`PackerErrorKind`] describes the errors that can be returned for a Packer
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum PackerErrorKind {
    /// Conversion from `{from}` to `{to}` failed: `{source}`
    Conversion {
        to: &'static str,
        from: &'static str,
        source: std::num::TryFromIntError,
    },
    /// Sending crossbeam message failed: `id`: `{id:?}`, `data`: `{data:?}` : `{source}`
    SendingCrossbeamMessage {
        id: BlobId,
        data: Bytes,
        source: crossbeam_channel::SendError<(Bytes, BlobId)>,
    },
    /// Sending crossbeam data message failed: `data`: `{data:?}`, `index_pack`: `{index_pack:?}` : `{source}`
    SendingCrossbeamDataMessage {
        data: Bytes,
        index_pack: Box<IndexPack>,
        source: crossbeam_channel::SendError<(Bytes, IndexPack)>,
    },
}

pub(crate) type PackerResult<T> = Result<T, Box<PackerErrorKind>>;

pub(super) mod constants {
    use std::time::Duration;

    /// Kilobyte in bytes
    pub(super) const KB: u32 = 1024;
    /// Megabyte in bytes
    pub(super) const MB: u32 = 1024 * KB;
    /// The absolute maximum size of a pack: including headers it should not exceed 4 GB
    pub(super) const MAX_SIZE: u32 = 4076 * MB;
    /// The maximum number of blobs in a pack
    pub(super) const MAX_COUNT: u32 = 10_000;
    /// The maximum age of a pack
    pub(super) const MAX_AGE: Duration = Duration::from_secs(300);
}

/// The pack sizer is responsible for computing the size of the pack file.
#[derive(Debug, Clone, Copy)]
pub struct PackSizer {
    /// The default size of a pack file.
    default_size: u32,
    /// The grow factor of a pack file.
    grow_factor: u32,
    /// The size limit of a pack file.
    size_limit: u32,
    /// The current size of a pack file.
    current_size: u64,
    /// The minimum pack size tolerance in percent before a repack is triggered.
    min_packsize_tolerate_percent: u32,
    /// The maximum pack size tolerance in percent before a repack is triggered.
    max_packsize_tolerate_percent: u32,
}

impl PackSizer {
    /// Creates a new `PackSizer` from a config file.
    ///
    /// # Arguments
    ///
    /// * `config` - The config file.
    /// * `blob_type` - The blob type.
    /// * `current_size` - The current size of the pack file.
    ///
    /// # Returns
    ///
    /// A new `PackSizer`.
    #[must_use]
    pub fn from_config(config: &ConfigFile, blob_type: BlobType, current_size: u64) -> Self {
        let (default_size, grow_factor, size_limit) = config.packsize(blob_type);
        let (min_packsize_tolerate_percent, max_packsize_tolerate_percent) =
            config.packsize_ok_percents();
        Self {
            default_size,
            grow_factor,
            size_limit,
            current_size,
            min_packsize_tolerate_percent,
            max_packsize_tolerate_percent,
        }
    }

    /// Creates a new `PackSizer` with a fixed size
    ///
    /// # Arguments
    ///
    /// * `size` - The fixed size to use.
    ///
    /// # Returns
    ///
    /// A new `PackSizer`.
    #[must_use]
    pub fn fixed(size: u32) -> Self {
        Self {
            default_size: size,
            grow_factor: 0,
            size_limit: size,
            current_size: 0,
            min_packsize_tolerate_percent: 100,
            max_packsize_tolerate_percent: 100,
        }
    }

    /// Computes the size of the pack file.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn pack_size(&self) -> u32 {
        let size = if self.grow_factor == 0 {
            self.default_size
        } else {
            // The cast actually shouldn't pose any problems.
            // `current_size` is `u64`, the maximum value is `2^64-1`.
            // `isqrt(2^64-1) = 2^32-1` which fits into a `u32`. (@aawsome)
            self.current_size.integer_sqrt() as u32 * self.grow_factor + self.default_size
        };
        size.min(self.size_limit).min(constants::MAX_SIZE)
    }

    /// Evaluates whether the given size is not too small or too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    pub fn size_ok(&self, size: u32) -> bool {
        !self.is_too_small(size) && !self.is_too_large(size)
    }

    /// Evaluates whether the given size is too small
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    pub fn is_too_small(&self, size: u32) -> bool {
        let target_size = self.pack_size();
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        u64::from(size) * 100
            < u64::from(target_size) * u64::from(self.min_packsize_tolerate_percent)
    }

    /// Evaluates whether the given size is too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    pub fn is_too_large(&self, size: u32) -> bool {
        let target_size = self.pack_size();
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        u64::from(size) * 100
            > u64::from(target_size) * u64::from(self.max_packsize_tolerate_percent)
    }

    /// Adds the given size to the current size.
    ///
    /// # Arguments
    ///
    /// * `added` - The size to add
    ///
    /// # Panics
    ///
    /// * If the size is too large
    pub fn add_size(&mut self, added: u32) {
        self.current_size += u64::from(added);
    }
}

/// The `Packer` is responsible for packing blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend type.
#[allow(missing_debug_implementations)]
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub struct Packer<BE: DecryptWriteBackend> {
    /// The raw packer wrapped in an `Arc` and `RwLock`.
    // This is a hack: raw_packer and indexer are only used in the add_raw() method.
    // TODO: Refactor as actor, like the other add() methods
    raw_packer: Arc<RwLock<RawPacker<BE>>>,
    /// The shared indexer containing the backend.
    indexer: SharedIndexer<BE>,
    /// The sender to send blobs to the raw packer.
    sender: Sender<(Bytes, BlobId)>,
    /// The receiver to receive the status from the raw packer.
    finish: Receiver<RusticResult<PackerStats>>,
}

impl<BE: DecryptWriteBackend> Packer<BE> {
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
        indexer: SharedIndexer<BE>,
        pack_sizer: PackSizer,
    ) -> RusticResult<Self> {
        let raw_packer = Arc::new(RwLock::new(RawPacker::new(
            be.clone(),
            blob_type,
            indexer.clone(),
            pack_sizer,
        )));

        let (tx, rx) = bounded(0);
        let (finish_tx, finish_rx) = bounded::<RusticResult<PackerStats>>(0);
        let packer = Self {
            raw_packer: raw_packer.clone(),
            indexer: indexer.clone(),
            sender: tx,
            finish: finish_rx,
        };

        let _join_handle = std::thread::spawn(move || {
            scope(|scope| {
                let status = rx
                    .into_iter()
                    .readahead_scoped(scope)
                    // early check if id is already contained
                    .filter(|(_, id)| !indexer.read().unwrap().has(id))
                    .filter(|(_, id)| !raw_packer.read().unwrap().has(id))
                    .readahead_scoped(scope)
                    .parallel_map_scoped(scope, |(data, id): (Bytes, BlobId)| {
                        let (data, data_len, uncompressed_length) = be.process_data(&data)?;
                        Ok((data, id, u64::from(data_len), uncompressed_length))
                    })
                    .readahead_scoped(scope)
                    // check again if id is already contained
                    // TODO: We may still save duplicate blobs - the indexer is only updated when the packfile write has completed
                    .filter(|res| {
                        res.as_ref()
                            .map_or_else(|_| true, |(_, id, _, _)| !indexer.read().unwrap().has(id))
                    })
                    .try_for_each(|item: RusticResult<_>| -> RusticResult<()> {
                        let (data, id, data_len, ul) = item?;
                        raw_packer
                            .write()
                            .unwrap()
                            .add_raw(&data, &id, data_len, ul)
                    })
                    .and_then(|()| raw_packer.write().unwrap().finalize());
                _ = finish_tx.send(status);
            })
            .unwrap();
        });

        Ok(packer)
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
        self.sender.send((data, id)).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Sending crossbeam message failed: `id`: `{id}`",
                err,
            )
            .ask_report()
            .attach_context("id", id.to_hex().to_string())
        })?;
        Ok(())
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
        if self.indexer.read().unwrap().has(id) {
            Ok(())
        } else {
            self.raw_packer
                .write()
                .unwrap()
                .add_raw(data, id, data_len, uncompressed_length)
        }
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
        self.finish
            .recv()
            .expect("Should be able to receive from channel to finalize packer.")
    }
}

// TODO: add documentation!
#[derive(Default, Debug, Clone, Copy)]
pub struct PackerStats {
    /// The number of blobs added
    blobs: u64,
    /// The number of data blobs added
    data: u64,
    /// The number of packed data blobs added
    data_packed: u64,
}

impl PackerStats {
    /// Adds the stats to the summary
    ///
    /// # Arguments
    ///
    /// * `summary` - The summary to add to
    /// * `tpe` - The blob type
    ///
    /// # Panics
    ///
    /// * If the blob type is invalid
    pub fn apply(self, summary: &mut SnapshotSummary, tpe: BlobType) {
        summary.data_added += self.data;
        summary.data_added_packed += self.data_packed;
        match tpe {
            BlobType::Tree => {
                summary.tree_blobs += self.blobs;
                summary.data_added_trees += self.data;
                summary.data_added_trees_packed += self.data_packed;
            }
            BlobType::Data => {
                summary.data_blobs += self.blobs;
                summary.data_added_files += self.data;
                summary.data_added_files_packed += self.data_packed;
            }
        }
    }
}

/// The `RawPacker` is responsible for packing blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend type.
#[allow(missing_debug_implementations, clippy::module_name_repetitions)]
pub(crate) struct RawPacker<BE: DecryptWriteBackend> {
    /// The backend to write to.
    be: BE,
    /// The blob type to pack.
    blob_type: BlobType,
    /// The file to write to
    file: BytesMut,
    /// The size of the file
    size: u32,
    /// The number of blobs in the pack
    count: u32,
    /// The time the pack was created
    created: SystemTime,
    /// The index of the pack
    index: IndexPack,
    /// The actor to write the pack file
    file_writer: Option<Actor>,
    /// The pack sizer
    pack_sizer: PackSizer,
    /// The packer stats
    stats: PackerStats,
}

impl<BE: DecryptWriteBackend> RawPacker<BE> {
    /// Creates a new `RawPacker`.
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
    fn new(be: BE, blob_type: BlobType, indexer: SharedIndexer<BE>, pack_sizer: PackSizer) -> Self {
        let file_writer = Some(Actor::new(
            FileWriterHandle {
                be: be.clone(),
                indexer,
                cacheable: blob_type.is_cacheable(),
            },
            1,
            1,
        ));

        Self {
            be,
            blob_type,
            file: BytesMut::new(),
            size: 0,
            count: 0,
            created: SystemTime::now(),
            index: IndexPack::default(),
            file_writer,
            pack_sizer,
            stats: PackerStats::default(),
        }
    }

    /// Saves the packfile and returns the stats
    ///
    /// # Errors
    ///
    /// * If the packfile could not be saved
    fn finalize(&mut self) -> RusticResult<PackerStats> {
        self.save().map_err(|err| {
            err.overwrite_kind(ErrorKind::Internal)
                .prepend_guidance_line("Failed to save packfile. Data may be lost.")
                .ask_report()
        })?;

        self.file_writer.take().unwrap().finalize()?;

        Ok(std::mem::take(&mut self.stats))
    }

    /// Writes the given data to the packfile.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to write.
    ///
    /// # Returns
    ///
    /// The number of bytes written.
    fn write_data(&mut self, data: &[u8]) -> PackerResult<u32> {
        let len = data
            .len()
            .try_into()
            .map_err(|err| PackerErrorKind::Conversion {
                to: "u32",
                from: "usize",
                source: err,
            })?;
        self.file.extend_from_slice(data);
        self.size += len;
        Ok(len)
    }

    /// Adds the already compressed/encrypted blob to the packfile without any check
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
    /// * If converting the data length to u64 fails
    fn add_raw(
        &mut self,
        data: &[u8],
        id: &BlobId,
        data_len: u64,
        uncompressed_length: Option<NonZeroU32>,
    ) -> RusticResult<()> {
        if self.has(id) {
            return Ok(());
        }
        self.stats.blobs += 1;

        self.stats.data += data_len;

        let data_len_packed: u64 = data.len().try_into().map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to convert data length `{length}` to u64.",
                err,
            )
            .attach_context("length", data.len().to_string())
        })?;

        self.stats.data_packed += data_len_packed;

        let size_limit = self.pack_sizer.pack_size();

        let offset = self.size;

        let len = self.write_data(data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to write data to packfile for blob `{id}`.",
                err,
            )
            .attach_context("id", id.to_string())
            .attach_context("size_limit", size_limit.to_string())
            .attach_context("data_length_packed", data_len_packed.to_string())
        })?;

        self.index
            .add(*id, self.blob_type, offset, len, uncompressed_length);

        self.count += 1;

        // check if PackFile needs to be saved
        let elapsed = self.created.elapsed().unwrap_or_else(|err| {
            warn!("couldn't get elapsed time from system time: {err:?}");
            Duration::ZERO
        });

        if self.count >= constants::MAX_COUNT
            || self.size >= size_limit
            || elapsed >= constants::MAX_AGE
        {
            self.pack_sizer.add_size(self.index.pack_size());
            self.save()?;
            self.size = 0;
            self.count = 0;
            self.created = SystemTime::now();
        }
        Ok(())
    }

    /// Writes header and length of header to packfile
    ///
    /// # Errors
    ///
    /// * If converting the header length to u32 fails
    /// * If the header could not be written
    fn write_header(&mut self) -> RusticResult<()> {
        // compute the pack header
        let data = PackHeaderRef::from_index_pack(&self.index)
            .to_binary()
            .map_err(|err| -> Box<RusticError> {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to convert pack header `{index_pack_id}` to binary representation.",
                    err,
                )
                .attach_context("index_pack_id", self.index.id.to_string())
            })?;

        // encrypt and write to pack file
        let data = self.be.key().encrypt_data(&data)?;

        let headerlen: u32 = data.len().try_into().map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to convert header length `{length}` to u32.",
                err,
            )
            .attach_context("length", data.len().to_string())
        })?;

        // write header to pack file
        _ = self.write_data(&data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to write header with length `{length}` to packfile.",
                err,
            )
            .attach_context("length", headerlen.to_string())
        })?;

        // convert header length to binary representation
        let binary_repr = PackHeaderLength::from_u32(headerlen)
            .to_binary()
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to convert header length `{length}` to binary representation.",
                    err,
                )
                .attach_context("length", headerlen.to_string())
            })?;

        // finally write length of header unencrypted to pack file
        _ = self.write_data(&binary_repr).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to write header length `{length}` to packfile.",
                err,
            )
            .attach_context("length", headerlen.to_string())
        })?;

        Ok(())
    }

    /// Saves the packfile
    ///
    /// # Errors
    ///
    /// If the header could not be written
    ///
    /// # Errors
    ///
    /// * If converting the header length to u32 fails
    /// * If the header could not be written
    fn save(&mut self) -> RusticResult<()> {
        if self.size == 0 {
            return Ok(());
        }

        self.write_header()?;

        // write file to backend
        let index = std::mem::take(&mut self.index);
        let file = std::mem::replace(&mut self.file, BytesMut::new());
        self.file_writer
            .as_ref()
            .unwrap()
            .send((file.into(), index))
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to send packfile to file writer.",
                    err,
                )
            })?;

        Ok(())
    }

    fn has(&self, id: &BlobId) -> bool {
        self.index.blobs.iter().any(|b| &b.id == id)
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
    indexer: SharedIndexer<BE>,
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
        index.time = Some(Timestamp::now());
        Ok(index)
    }

    fn index(&self, index: IndexPack) -> RusticResult<()> {
        self.indexer.write().unwrap().add(index)?;
        Ok(())
    }
}

// TODO: add documentation
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
    fn send(&self, load: (Bytes, IndexPack)) -> PackerResult<()> {
        self.sender.send(load.clone()).map_err(|err| {
            PackerErrorKind::SendingCrossbeamDataMessage {
                data: load.0,
                index_pack: Box::new(load.1),
                source: err,
            }
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CopyPackBlobs {
    pub pack_id: PackId,
    pub locations: BlobLocations<BlobId>,
}

impl CopyPackBlobs {
    pub fn from_index_blob(pack_id: PackId, blob: IndexBlob) -> Self {
        Self {
            pack_id,
            locations: BlobLocations::from_blob_location(blob.location, blob.id),
        }
    }

    pub fn from_index_entry(entry: IndexEntry, id: BlobId) -> Self {
        Self {
            pack_id: entry.pack,
            locations: BlobLocations::from_blob_location(entry.location, id),
        }
    }

    #[allow(clippy::result_large_err)]
    /// coalesce two `RepackBlobs` if possible
    pub fn coalesce(self, other: Self) -> Result<Self, (Self, Self)> {
        if self.pack_id == other.pack_id && self.locations.can_coalesce(&other.locations) {
            Ok(Self {
                pack_id: self.pack_id,
                locations: self.locations.append(other.locations),
            })
        } else {
            Err((self, other))
        }
    }
}

/// The `BlobCopier` is responsible for copying or repacking blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend to read from.
#[allow(missing_debug_implementations)]
pub struct BlobCopier<BE>
where
    BE: DecryptFullBackend,
{
    /// The backend to read from.
    be_src: BE,
    /// The packer to write to.
    packer: Packer<BE>,
    /// The size limit of the pack file.
    size_limit: u32,
    /// the blob type
    blob_type: BlobType,
}

impl<BE: DecryptFullBackend> BlobCopier<BE> {
    /// Creates a new `BlobCopier`.
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
        be_src: BE,
        be_dst: BE,
        blob_type: BlobType,
        indexer: SharedIndexer<BE>,
        pack_sizer: PackSizer,
    ) -> RusticResult<Self> {
        let packer = Packer::new(be_dst, blob_type, indexer, pack_sizer)?;
        let size_limit = pack_sizer.pack_size();
        Ok(Self {
            be_src,
            packer,
            size_limit,
            blob_type,
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
    pub fn copy_fast(&self, pack_blobs: CopyPackBlobs, p: &Progress) -> RusticResult<()> {
        let offset = pack_blobs.locations.offset;
        let data = self.be_src.read_partial(
            FileType::Pack,
            &pack_blobs.pack_id,
            self.blob_type.is_cacheable(),
            offset,
            pack_blobs.locations.length,
        )?;

        // TODO: write in parallel
        for (blob, blob_id) in pack_blobs.locations.blobs {
            let start = usize::try_from(blob.offset - offset)
                .expect("convert from u32 to usize should not fail!");
            let end = usize::try_from(blob.offset + blob.length - offset)
                .expect("convert from u32 to usize should not fail!");
            self.packer
                .add_raw(
                    &data[start..end],
                    &blob_id,
                    u64::from(blob.length),
                    blob.uncompressed_length,
                )
                .map_err(|err| {
                    err.overwrite_kind(ErrorKind::Internal)
                        .prepend_guidance_line(
                            "Failed to fast-add (unchecked) blob `{blob_id}` to packfile.",
                        )
                        .attach_context("blob_id", blob_id.to_string())
                })?;
            p.inc(blob.length.into());
        }

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
    pub fn copy(&self, pack_blobs: CopyPackBlobs, p: &Progress) -> RusticResult<()> {
        let offset = pack_blobs.locations.offset;
        let read_data = self.be_src.read_partial(
            FileType::Pack,
            &pack_blobs.pack_id,
            self.blob_type.is_cacheable(),
            offset,
            pack_blobs.locations.length,
        )?;

        // TODO: write in parallel
        for (blob, blob_id) in pack_blobs.locations.blobs {
            let start = usize::try_from(blob.offset - offset)
                .expect("convert from u32 to usize should not fail!");
            let end = usize::try_from(blob.offset + blob.length - offset)
                .expect("convert from u32 to usize should not fail!");
            let data = self
                .be_src
                .read_encrypted_from_partial(&read_data[start..end], blob.uncompressed_length)?;

            self.packer.add(data, blob_id).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to add blob to packfile.",
                    err,
                )
            })?;
            p.inc(blob.length.into());
        }

        Ok(())
    }

    /// Finalizes the repacker and returns the stats
    pub fn finalize(self) -> RusticResult<PackerStats> {
        self.packer.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_ron_snapshot;

    #[test]
    fn pack_sizers() {
        let config = ConfigFile {
            treepack_size_limit: Some(5 * 1024 * 1024),
            ..Default::default()
        };
        let mut pack_sizers = [
            PackSizer::from_config(&config, BlobType::Tree, 0),
            PackSizer::from_config(&config, BlobType::Data, 0),
            PackSizer::fixed(12345),
        ];

        let output: Vec<_> = [
            0,
            10,
            1000,
            100_000,
            100_000,
            100_000,
            10_000_000,
            10_000_000,
            1_000_000_000,
            1_000_000_000,
        ]
        .into_iter()
        .map(|i| {
            pack_sizers
                .iter_mut()
                .map(|ps| {
                    ps.add_size(i);
                    (ps.current_size, ps.pack_size())
                })
                .collect::<Vec<_>>()
        })
        .collect();

        assert_ron_snapshot!(output);
    }
}
