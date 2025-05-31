use std::{
    num::NonZeroU32,
    time::{Duration, SystemTime},
};

use bytes::{Bytes, BytesMut};
use integer_sqrt::IntegerSquareRoot;
use log::warn;

use crate::{
    blob::{BlobId, BlobType},
    crypto::CryptoKey,
    error::{ErrorKind, RusticError, RusticResult},
    repofile::{
        configfile::ConfigFile,
        indexfile::IndexPack,
        packfile::{PackHeaderLength, PackHeaderRef},
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
}

pub(crate) type PackerResult<T> = Result<T, PackerErrorKind>;

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
    #[must_use]
    // The cast actually shouldn't pose any problems.
    // `current_size` is `u64`, the maximum value is `2^64-1`.
    // `isqrt(2^64-1) = 2^32-1` which fits into a `u32`. (@aawsome)
    #[allow(clippy::cast_possible_truncation)]
    pub fn pack_size(&self) -> u32 {
        (self.current_size.integer_sqrt() as u32 * self.grow_factor + self.default_size)
            .min(self.size_limit)
            .min(constants::MAX_SIZE)
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
    fn add_size(&mut self, added: u32) {
        self.current_size += u64::from(added);
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
pub(crate) struct Packer<C> {
    /// the
    key: C,
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
    /// The pack sizer
    pub pack_sizer: PackSizer,
    /// The packer stats
    pub stats: PackerStats,
}

impl<C: CryptoKey> Packer<C> {
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
    pub fn new(key: C, blob_type: BlobType, config: &ConfigFile, total_size: u64) -> Self {
        let pack_sizer = PackSizer::from_config(config, blob_type, total_size);

        Self {
            key,
            blob_type,
            file: BytesMut::new(),
            size: 0,
            count: 0,
            created: SystemTime::now(),
            index: IndexPack::default(),
            pack_sizer,
            stats: PackerStats::default(),
        }
    }

    /// Saves the packfile and returns the stats
    ///
    /// # Errors
    ///
    /// * If the packfile could not be saved
    pub fn finalize(&mut self) -> RusticResult<(Option<(Bytes, IndexPack)>, PackerStats)> {
        let stats = std::mem::take(&mut self.stats);

        Ok((self.save()?, stats))
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
    pub fn add(
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

        let offset = self.size;

        let len = self.write_data(data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to write data to packfile for blob `{id}`.",
                err,
            )
            .attach_context("id", id.to_string())
            .attach_context("data_length_packed", data_len_packed.to_string())
        })?;

        self.index
            .add(*id, self.blob_type, offset, len, uncompressed_length);

        self.count += 1;

        Ok(())
    }

    pub fn needs_save(&self, size_limit: Option<u32>) -> bool {
        if self.size == 0 {
            return false;
        }

        let size_limit = size_limit.unwrap_or_else(|| self.pack_sizer.pack_size());

        // check if PackFile needs to be saved
        let elapsed = self.created.elapsed().unwrap_or_else(|err| {
            warn!("couldn't get elapsed time from system time: {err:?}");
            Duration::ZERO
        });

        self.count >= constants::MAX_COUNT
            || self.size >= size_limit
            || elapsed >= constants::MAX_AGE
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
        let data = self.key.encrypt_data(&data)?;

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
    pub fn save_if_needed(
        &mut self,
        size_limit: Option<u32>,
    ) -> RusticResult<Option<(Bytes, IndexPack)>> {
        if !self.needs_save(size_limit) {
            return Ok(None);
        }

        self.save()
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
    pub fn save(&mut self) -> RusticResult<Option<(Bytes, IndexPack)>> {
        self.created = SystemTime::now();
        self.count = 0;

        if self.size == 0 {
            return Ok(None);
        }

        self.write_header()?;
        // prepare everything for write to the backend
        let file = std::mem::take(&mut self.file).into();
        let index = std::mem::take(&mut self.index);

        self.size = 0;

        Ok(Some((file, index)))
    }

    pub fn has(&self, id: &BlobId) -> bool {
        self.index.blobs.iter().any(|b| &b.id == id)
    }
}
