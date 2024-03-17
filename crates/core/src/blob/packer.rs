use integer_sqrt::IntegerSquareRoot;
use log::{error, warn};

use std::{
    fmt::Debug,
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, SystemTime},
};

use parking_lot::RwLock;

use bytes::{Bytes, BytesMut};
use chrono::Local;
use crossbeam_channel::{bounded, Receiver, Sender};
use pariter::{scope, IteratorExt};

use crate::{
    backend::{
        decrypt::{DecryptFullBackend, DecryptWriteBackend},
        FileType,
    },
    blob::BlobType,
    crypto::{hasher::hash, CryptoKey},
    error::{MultiprocessingErrorKind, PackerErrorKind, RusticErrorKind, RusticResult},
    id::Id,
    index::indexer::SharedIndexer,
    repofile::{
        configfile::ConfigFile,
        indexfile::{IndexBlob, IndexPack},
        packfile::{PackHeaderLength, PackHeaderRef},
        snapshotfile::SnapshotSummary,
    },
};

pub(crate) type SharedPacker<BE> = Arc<RwLock<RawPacker<BE>>>;

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

    /// Computes the size of the pack file.
    // The cast actually shouldn't pose any problems.
    // `current_size` is `u64`, the maximum value is `2^64-1`.
    // `isqrt(2^64-1) = 2^32-1` which fits into a `u32`. (@aawsome)
    #[allow(clippy::cast_possible_truncation)]
    pub fn pack_size(&self) -> RusticResult<u32> {
        Ok(calculate_pack_size(
            u32::try_from(self.current_size).map_err(|err| {
                PackerErrorKind::IntConversionFailedInPackSizeCalculation {
                    value: self.current_size,
                    comment: err.to_string(),
                }
            })?,
            self.grow_factor,
            self.default_size,
            self.size_limit,
        ))
    }

    /// Evaluates whether the given size is not too small or too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    ///
    /// # Returns
    ///
    /// `True` if the given size is not too small _and_ too large,
    /// `False` otherwise
    pub fn size_ok(&self, size: u32) -> RusticResult<bool> {
        Ok(!self.is_too_small(size)? && !self.is_too_large(size)?)
    }

    /// Evaluates whether the given size is too small
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    pub fn is_too_small(&self, size: u32) -> RusticResult<bool> {
        let target_size = self.pack_size()?;
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        Ok(u64::from(size).checked_mul(100).ok_or(
            PackerErrorKind::IntConversionFailedInPackSizeCalculation {
                value: u64::from(size),
                comment: "overflow".into(),
            },
        )? < u64::from(target_size)
            .checked_mul(u64::from(self.min_packsize_tolerate_percent))
            .ok_or(PackerErrorKind::IntConversionFailedInPackSizeCalculation {
                value: u64::from(target_size),
                comment: "overflow".into(),
            })?)
    }

    /// Evaluates whether the given size is too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    pub fn is_too_large(&self, size: u32) -> RusticResult<bool> {
        let target_size = self.pack_size()?;
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        Ok(u64::from(size).checked_mul(100).ok_or(
            PackerErrorKind::IntConversionFailedInPackSizeCalculation {
                value: u64::from(size),
                comment: "overflow".into(),
            },
        )? > u64::from(target_size)
            .checked_mul(u64::from(self.max_packsize_tolerate_percent))
            .ok_or(PackerErrorKind::IntConversionFailedInPackSizeCalculation {
                value: u64::from(target_size),
                comment: "overflow".into(),
            })?)
    }

    /// Adds the given size to the current size.
    ///
    /// # Arguments
    ///
    /// * `added` - The size to add
    fn add_size(&mut self, added: u32) -> RusticResult<()> {
        if added > self.size_limit {
            return Err(PackerErrorKind::SizeLimitExceeded(added).into());
        }
        if let Some(value) = self.current_size.checked_add(u64::from(added)) {
            self.current_size = value;
        } else {
            return Err(PackerErrorKind::AddingSizeToCurrentSizeFailed {
                current_size: self.current_size,
                to_be_added: added,
            }
            .into());
        }
        Ok(())
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
    /// The raw packer wrapped in an Arc and RwLock.
    // This is a hack: raw_packer and indexer are only used in the add_raw() method.
    // TODO: Refactor as actor, like the other add() methods
    raw_packer: SharedPacker<BE>,
    /// The shared indexer containing the backend.
    indexer: SharedIndexer<BE>,
    /// The sender to send blobs to the raw packer.
    sender: Sender<(Bytes, Id, Option<u32>)>,
    /// The receiver to receive the status from the raw packer.
    finish: Receiver<RusticResult<PackerStats>>,
}

impl<BE: DecryptWriteBackend + Debug> Debug for Packer<BE> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packer")
            .field("raw_packer", &self.raw_packer)
            .field("indexer", &self.indexer)
            .field("sender", &self.sender)
            .field("finish", &self.finish)
            .finish()
    }
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
    /// * [`PackerErrorKind::SendingCrossbeamMessageFailed`] - If sending the message to the raw packer fails.
    /// * [`PackerErrorKind::IntConversionFailed`] - If converting the data length to u64 fails
    ///
    /// [`PackerErrorKind::SendingCrossbeamMessageFailed`]: crate::error::PackerErrorKind::SendingCrossbeamMessageFailed
    /// [`PackerErrorKind::IntConversionFailed`]: crate::error::PackerErrorKind::IntConversionFailed
    #[allow(clippy::unnecessary_wraps)]
    pub fn new(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer<BE>,
        config: &ConfigFile,
        total_size: u64,
    ) -> RusticResult<Self> {
        let raw_packer = Arc::new(RwLock::new(RawPacker::new(
            be.clone(),
            blob_type,
            indexer.clone(),
            config,
            total_size,
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
                    .filter(|(_, id, _)| !indexer.read().has(id))
                    .filter(|(_, id, _)| !raw_packer.read().has(id))
                    .readahead_scoped(scope)
                    .parallel_map_scoped(
                        scope,
                        |(data, id, size_limit): (Bytes, Id, Option<u32>)| {
                            let (data, data_len, uncompressed_length) = be.process_data(&data)?;
                            Ok((
                                data,
                                id,
                                u64::from(data_len),
                                uncompressed_length,
                                size_limit,
                            ))
                        },
                    )
                    .readahead_scoped(scope)
                    // check again if id is already contained
                    // TODO: We may still save duplicate blobs - the indexer is only updated when the packfile write has completed
                    .filter(|res| {
                            .map_or_else(|_| true, |(_, id, _, _, _)| !indexer.read().has(id))
                    })
                    .try_for_each(|item: RusticResult<_>| {
                        let (data, id, data_len, ul, size_limit) = item?;
                        raw_packer
                            .write()
                            .add_raw(&data, &id, data_len, ul, size_limit)
                    })
                    .and_then(|()| raw_packer.write().finalize());
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
    /// * [`PackerErrorKind::SendingCrossbeamMessageFailed`] - If sending the message to the raw packer fails.
    ///
    /// [`PackerErrorKind::SendingCrossbeamMessageFailed`]: crate::error::PackerErrorKind::SendingCrossbeamMessageFailed
    pub fn add(&self, data: Bytes, id: Id) -> RusticResult<()> {
        // compute size limit based on total size and size bounds
        self.add_with_sizelimit(data, id, None)
    }

    /// Adds the blob to the packfile, allows specifying a size limit for the pack file
    ///
    /// # Arguments
    ///
    /// * `data` - The blob data
    /// * `id` - The blob id
    /// * `size_limit` - The size limit for the pack file
    ///
    /// # Errors
    ///
    /// * [`ChannelErrorKind::SendingCrossbeamMessageFailedWithBytes`] - If sending the message to the raw packer fails.
    ///
    /// [`PackerErrorKind::SendingCrossbeamMessageFailed`]: crate::error::PackerErrorKind::SendingCrossbeamMessageFailed
    fn add_with_sizelimit(&self, data: Bytes, id: Id, size_limit: Option<u32>) -> RusticResult<()> {
        self.sender
            .send((data, id, size_limit))
            .map_err(PackerErrorKind::SendingCrossbeamMessageFailed)?;
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
    /// If the blob is already present in the index
    /// If sending the message to the raw packer fails.
    fn add_raw(
        &self,
        data: &[u8],
        id: &Id,
        data_len: u64,
        uncompressed_length: Option<NonZeroU32>,
        size_limit: Option<u32>,
    ) -> RusticResult<()> {
        // only add if this blob is not present
        if self.indexer.read().has(id) {
            Ok(())
        } else {
            self.raw_packer
                .write()
                .add_raw(data, id, data_len, uncompressed_length, size_limit)
        }
    }

    /// Finalizes the packer and does cleanup
    ///
    /// # Errors
    ///
    /// [`MultiprocessingErrorKind::SenderDropped`] - If receiving the message from the raw packer fails.
    ///
    /// # Returns
    ///
    /// The packer stats
    pub fn finalize(self) -> RusticResult<PackerStats> {
        // cancel channel
        drop(self.sender);

        // wait for items in channel to be processed
        self.finish.recv().map_or_else(
            |_| {
                error!("sender has been dropped unexpectedly.");
                Err(MultiprocessingErrorKind::SenderDropped.into())
            },
            |stats| stats,
        )
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
    /// # Errors
    ///
    /// If the summary could not be updated
    /// If the addition of the stats to the summary overflowed
    ///
    /// # Returns
    ///
    /// The updated summary
    pub fn apply(self, summary: &mut SnapshotSummary, tpe: BlobType) -> RusticResult<()> {
        let mut errors = vec![];
        if let Some(val) = summary.data_added.checked_add(self.data) {
            summary.data_added = val;
        } else {
            warn!("data_added overflowed");
            errors.push(PackerErrorKind::DataAddedOverflowed {
                data_added: summary.data_added,
                data: self.data,
            });
        }
        if let Some(val) = summary.data_added_packed.checked_add(self.data_packed) {
            summary.data_added_packed = val;
        } else {
            warn!("data_added_packed overflowed");
            errors.push(PackerErrorKind::DataAddedPackedOverflowed {
                data_added_packed: summary.data_added_packed,
                data_packed: self.data_packed,
            });
        };
        match tpe {
            BlobType::Tree => {
                if let Some(val) = summary.tree_blobs.checked_add(self.blobs) {
                    summary.tree_blobs = val;
                } else {
                    warn!("tree_blobs overflowed");

                    errors.push(PackerErrorKind::TreeBlobsOverflowed {
                        tree_blobs: summary.tree_blobs,
                        blobs: self.blobs,
                    });
                }
                if let Some(val) = summary.data_added_trees.checked_add(self.data) {
                    summary.data_added_trees = val;
                } else {
                    warn!("data_added_trees overflowed");
                    errors.push(PackerErrorKind::DataAddedTreesOverflowed {
                        data_added_trees: summary.data_added_trees,
                        data: self.data,
                    });
                }
                if let Some(val) = summary
                    .data_added_trees_packed
                    .checked_add(self.data_packed)
                {
                    summary.data_added_trees_packed = val;
                } else {
                    warn!("data_added_trees_packed overflowed");
                    errors.push(PackerErrorKind::DataAddedTreesPackedOverflowed {
                        data_added_trees_packed: summary.data_added_trees_packed,
                        data_packed: self.data_packed,
                    });
                };
            }
            BlobType::Data => {
                if let Some(val) = summary.data_blobs.checked_add(self.blobs) {
                    summary.data_blobs = val;
                } else {
                    warn!("data_blobs overflowed");
                    errors.push(PackerErrorKind::DataBlobsOverflowed {
                        data_blobs: summary.data_blobs,
                        blobs: self.blobs,
                    });
                };
                if let Some(val) = summary.data_added_files.checked_add(self.data) {
                    summary.data_added_files = val;
                } else {
                    warn!("data_added_files overflowed");
                    errors.push(PackerErrorKind::DataAddedFilesOverflowed {
                        data_added_files: summary.data_added_files,
                        data: self.data,
                    });
                };
                if let Some(val) = summary
                    .data_added_files_packed
                    .checked_add(self.data_packed)
                {
                    summary.data_added_files_packed = val;
                } else {
                    warn!("data_added_files_packed overflowed");
                    errors.push(PackerErrorKind::DataAddedFilesPackedOverflowed {
                        data_added_files_packed: summary.data_added_files_packed,
                        data_packed: self.data_packed,
                    });
                };
            }
        }
        if !errors.is_empty() {
            return Err(PackerErrorKind::MultipleFromSummary(errors).into());
        }
        Ok(())
    }
}

/// The `RawPacker` is responsible for packing blobs into pack files.
///
/// # Type Parameters
///
/// * `BE` - The backend type.
#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Debug)]
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
    fn new(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer<BE>,
        config: &ConfigFile,
        total_size: u64,
    ) -> Self {
        let file_writer = Some(Actor::new(
            FileWriterHandle {
                be: be.clone(),
                indexer,
                cacheable: blob_type.is_cacheable(),
            },
            1,
            1,
        ));

        let pack_sizer = PackSizer::from_config(config, blob_type, total_size);

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
    /// If the packfile could not be saved
    fn finalize(&mut self) -> RusticResult<PackerStats> {
        self.save()?;
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
    fn write_data(&mut self, data: &[u8]) -> RusticResult<u32> {
        let len = data
            .len()
            .try_into()
            .map_err(PackerErrorKind::IntConversionFailed)?;
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
    /// * [`PackerErrorKind::IntConversionFailed`] - If converting the data length to u64 fails
    ///
    /// [`PackerErrorKind::IntConversionFailed`]: crate::error::PackerErrorKind::IntConversionFailed
    fn add_raw(
        &mut self,
        data: &[u8],
        id: &Id,
        data_len: u64,
        uncompressed_length: Option<NonZeroU32>,
        size_limit: Option<u32>,
    ) -> RusticResult<()> {
        if self.has(id) {
            return Ok(());
        }
        self.stats.blobs += 1;
        self.stats.data += data_len;
        let data_len_packed: u64 = data
            .len()
            .try_into()
            .map_err(PackerErrorKind::IntConversionFailed)?;
        self.stats.data_packed += data_len_packed;

        let size_limit = size_limit.unwrap_or_else(|| self.pack_sizer.pack_size());
        let offset = self.size;
        let len = self.write_data(data)?;
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
    /// * [`PackerErrorKind::IntConversionFailed`] - If converting the header length to u32 fails
    /// * [`PackFileErrorKind::WritingBinaryRepresentationFailed`] - If the header could not be written
    ///
    /// [`PackerErrorKind::IntConversionFailed`]: crate::error::PackerErrorKind::IntConversionFailed
    /// [`PackFileErrorKind::WritingBinaryRepresentationFailed`]: crate::error::PackFileErrorKind::WritingBinaryRepresentationFailed
    fn write_header(&mut self) -> RusticResult<()> {
        // compute the pack header
        let data = PackHeaderRef::from_index_pack(&self.index).to_binary()?;

        // encrypt and write to pack file
        let data = self.be.key().encrypt_data(&data)?;

        let headerlen = data
            .len()
            .try_into()
            .map_err(PackerErrorKind::IntConversionFailed)?;
        _ = self.write_data(&data)?;

        // finally write length of header unencrypted to pack file
        _ = self.write_data(&PackHeaderLength::from_u32(headerlen).to_binary()?)?;

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
    /// * [`PackerErrorKind::IntConversionFailed`] - If converting the header length to u32 fails
    /// * [`PackFileErrorKind::WritingBinaryRepresentationFailed`] - If the header could not be written
    ///
    /// [`PackerErrorKind::IntConversionFailed`]: crate::error::PackerErrorKind::IntConversionFailed
    /// [`PackFileErrorKind::WritingBinaryRepresentationFailed`]: crate::error::PackFileErrorKind::WritingBinaryRepresentationFailed
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
            .send((file.into(), index))?;

        Ok(())
    }

    fn has(&self, id: &Id) -> bool {
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
    /// Processes the given data and writes it to the backend.
    ///
    /// # Arguments
    ///
    /// * `load` - The data to process.
    ///
    /// # Errors
    ///
    /// If writing the data to the backend fails.
    ///
    /// # Returns
    ///
    /// The index pack.
    fn process(&self, load: (Bytes, Id, IndexPack)) -> RusticResult<IndexPack> {
        let (file, id, mut index) = load;
        index.id = id;
        self.be
            .write_bytes(FileType::Pack, &id, self.cacheable, file)
            .map_err(RusticErrorKind::Backend)?;
        index.time = Some(Local::now());
        Ok(index)
    }

    fn index(&self, index: IndexPack) -> RusticResult<()> {
        self.indexer.write().add(index)?;
        Ok(())
    }
}

// TODO: add documentation
#[derive(Debug, Clone)]
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
    ///
    /// # Errors
    ///
    /// If sending the message to the actor fails.
    ///
    /// # Returns
    ///
    /// A new `Actor`.
    #[allow(clippy::unnecessary_wraps)]
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
                        (file, id, index)
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
    /// [`ChannelErrorKind::SendingCrossbeamMessageFailedForIndexPack`] - If sending the message to the actor fails.
    ///
    /// # Returns
    ///
    /// Ok if the message was sent successfully
    fn send(&self, load: (Bytes, IndexPack)) -> RusticResult<()> {
        self.sender
            .send(load)
            .map_err(PackerErrorKind::SendingCrossbeamMessageFailedForIndexPack)?;
        Ok(())
    }

    /// Finalizes the actor and does cleanup
    ///
    /// # Errors
    ///
    /// [`ChannelErrorKind::ReceivingCrossbeamMessageFailedForActorFinalizing`] - If receiving the message from the actor fails.
    ///
    /// # Returns
    ///
    /// Ok if the message was received successfully
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
    packer: Packer<BE>,
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
    /// If the Packer could not be created
    pub fn new(
        be: BE,
        blob_type: BlobType,
        indexer: SharedIndexer<BE>,
        config: &ConfigFile,
        total_size: u64,
    ) -> RusticResult<Self> {
        let packer = Packer::new(be.clone(), blob_type, indexer, config, total_size)?;
        let size_limit = PackSizer::from_config(config, blob_type, total_size).pack_size();
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
    /// If the blob could not be added
    /// If reading the blob from the backend fails
    pub fn add_fast(&self, pack_id: &Id, blob: &IndexBlob) -> RusticResult<()> {
        let data = self
            .be
            .read_partial(
                FileType::Pack,
                pack_id,
                blob.tpe.is_cacheable(),
                blob.offset,
                blob.length,
            )
            .map_err(RusticErrorKind::Backend)?;
        self.packer.add_raw(
            &data,
            &blob.id,
            0,
            blob.uncompressed_length,
            Some(self.size_limit),
        )?;
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
    /// If the blob could not be added
    /// If reading the blob from the backend fails
    pub fn add(&self, pack_id: &Id, blob: &IndexBlob) -> RusticResult<()> {
        let data = self.be.read_encrypted_partial(
            FileType::Pack,
            pack_id,
            blob.tpe.is_cacheable(),
            blob.offset,
            blob.length,
            blob.uncompressed_length,
        )?;
        self.packer
            .add_with_sizelimit(data, blob.id, Some(self.size_limit))?;
        Ok(())
    }

    /// Finalizes the repacker and returns the stats
    pub fn finalize(self) -> RusticResult<PackerStats> {
        self.packer.finalize()
    }
}

/// Calculates the pack size.
///
/// # Arguments
///
/// * `current_size` - The current size of the pack file.
/// * `grow_factor` - The grow factor.
/// * `default_size` - The default size.
/// * `size_limit` - The size limit.
///
/// # Returns
///
/// The pack size.
#[must_use]
#[inline]
pub fn calculate_pack_size(
    current_size: u32,
    grow_factor: u32,
    default_size: u32,
    size_limit: u32,
) -> u32 {
    current_size.integer_sqrt() * grow_factor
        + default_size.min(size_limit).min(constants::MAX_SIZE)
}

#[cfg(test)]
mod tests {

    use super::*;

    use rstest::{fixture, rstest};

    #[fixture]
    fn pack_sizer() -> PackSizer {
        let config = ConfigFile {
            max_packsize_tolerate_percent: Some(80),
            ..Default::default()
        };
        let blob_type = BlobType::Data;
        let current_size = 100;
        PackSizer::from_config(&config, blob_type, current_size)
    }

    #[test]
    fn test_pack_sizer_from_config_passes() {
        let config = ConfigFile::default();
        let blob_type = BlobType::Data;
        let (default_size, grow_factor, size_limit) = config.packsize(blob_type);
        let current_size = 0;
        let pack_sizer = PackSizer::from_config(&config, blob_type, current_size);
        let (min_packsize_tolerate_percent, max_packsize_tolerate_percent) =
            config.packsize_ok_percents();

        assert_eq!(pack_sizer.default_size, default_size, "default_size");
        assert_eq!(pack_sizer.grow_factor, grow_factor);
        assert_eq!(pack_sizer.size_limit, size_limit);
        assert_eq!(
            pack_sizer.min_packsize_tolerate_percent,
            min_packsize_tolerate_percent
        );
        assert_eq!(
            pack_sizer.max_packsize_tolerate_percent,
            max_packsize_tolerate_percent
        );
        assert_eq!(pack_sizer.current_size, current_size);
    }

    #[rstest]
    #[case(0.5f32, false, "size is too small, should be 'false'")]
    #[case(1.1f32, true, "size is ok, should be 'true'")]
    #[case(1_000_000.0f32, false, "size is too huge: should be 'false'")]
    fn test_compute_pack_size_ok_passes(
        pack_sizer: PackSizer,
        #[case] input: f32,
        #[case] expected: bool,
        #[case] comment: &str,
    ) -> RusticResult<()> {
        let size_limit = pack_sizer.pack_size()? * 30 / 100;

        #[allow(clippy::cast_possible_truncation)]
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_precision_loss)]
        let size = (input * size_limit as f32) as u32;

        assert_eq!(pack_sizer.size_ok(size)?, expected, "{comment}");

        Ok(())
    }
}
