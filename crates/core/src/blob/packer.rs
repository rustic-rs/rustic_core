use std::{
    num::NonZeroU32,
    time::{Duration, SystemTime},
};

use bytes::{Bytes, BytesMut};
use log::warn;

use crate::{
    blob::{BlobId, BlobType},
    crypto::{CryptoKey, hasher::hash},
    error::{ErrorKind, RusticError, RusticResult},
    repofile::{
        HeaderEntry,
        indexfile::IndexPack,
        packfile::{self, PackHeaderLength, PackHeaderRef},
        snapshotfile::SnapshotSummary,
    },
};

use super::pack_sizer::PackSizer;

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
    /// The maximum size used for padding
    pub(super) const MAX_PADDING: u32 = 64 * KB;
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
pub(crate) struct Packer<C, S> {
    /// the key to encrypt data
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
    pub pack_sizer: S,
    /// The packer stats
    pub stats: PackerStats,
    /// add a padding blob to stealthen the packsize
    add_padding: bool,
}

impl<C: CryptoKey, S: PackSizer> Packer<C, S> {
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
    pub fn new(key: C, pack_sizer: S, blob_type: BlobType, add_padding: bool) -> Self {
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
            add_padding,
        }
    }

    /// Saves the packfile and returns the stats
    ///
    /// # Errors
    ///
    /// * If the packfile could not be saved
    pub fn finalize(mut self) -> RusticResult<(Option<(Bytes, IndexPack)>, PackerStats)> {
        Ok((self.save()?, self.stats))
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
        let len: u32 = data
            .len()
            .try_into()
            .map_err(|err| PackerErrorKind::Conversion {
                to: "u32",
                from: "usize",
                source: err,
            })?;
        let data_len_packed: u64 = len.into();
        self.stats.data_packed += data_len_packed;
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
        self.stats.blobs += 1;
        self.stats.data += data_len;

        let offset = self.size;

        let len = self.write_data(data).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to write data to packfile for blob `{id}`.",
                err,
            )
            .attach_context("id", id.to_string())
        })?;

        self.index
            .add(*id, self.blob_type, offset, len, uncompressed_length);

        self.count += 1;

        Ok(())
    }

    /// Determines if the current pack should be saved.
    pub fn needs_save(&self) -> bool {
        if self.size == 0 {
            return false;
        }

        let size_limit = self.pack_sizer.pack_size().min(constants::MAX_SIZE);

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
    /// Saves the packfile if conditions for saving are fulfilled
    pub fn save_if_needed(&mut self) -> RusticResult<Option<(Bytes, IndexPack)>> {
        if !self.needs_save() {
            return Ok(None);
        }

        self.save()
    }

    /// Saves the packfile
    ///
    /// # Errors
    ///
    /// If the header could not be written
    pub fn save(&mut self) -> RusticResult<Option<(Bytes, IndexPack)>> {
        self.created = SystemTime::now();
        self.count = 0;

        if self.size == 0 {
            return Ok(None);
        }

        if self.add_padding {
            self.add_padding_blob()?;
        }
        self.write_header()?;
        // prepare everything for write to the backend
        let file = std::mem::take(&mut self.file).into();
        let index = std::mem::take(&mut self.index);
        self.pack_sizer.add_size(self.size);

        self.size = 0;

        Ok(Some((file, index)))
    }

    // Add a padding blob
    fn add_padding_blob(&mut self) -> RusticResult<()> {
        pub(super) const KB: u32 = 1024;
        pub(super) const MAX_PADDING: u32 = 64 * KB;

        // compute current size including the HeaderEntry and crypt overhead of the padding blob to-add
        let size = PackHeaderRef::from_index_pack(&self.index).pack_size()
            + HeaderEntry::ENTRY_LEN
            + packfile::constants::COMP_OVERHEAD;

        let padding_size = padding_size(size);

        // write padding blob
        let data = vec![
            0;
            padding_size
                .try_into()
                .expect("u32 should convert to usize")
        ];
        let id = BlobId(hash(&data));
        let data = self.key.encrypt_data(&data)?;
        let padding_size = padding_size.into();
        self.add(&data, &id, padding_size, None)?;

        // correct stats - padding should not contribute to blobs and data_added
        self.stats.blobs -= 1;
        self.stats.data -= padding_size;
        Ok(())
    }
}

fn padding_size(size: u32) -> u32 {
    // compute padding size. Note that we don't add zero-sized blobs here, i.e. padding_size is in 1..=MAX_PADDING.
    let padding = constants::MAX_PADDING - size % constants::MAX_PADDING;
    if padding == 0 {
        constants::MAX_PADDING
    } else {
        padding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padding_size() {
        assert_eq!(padding_size(1), constants::MAX_PADDING - 1);
        assert_eq!(padding_size(constants::MAX_PADDING - 1), 1);
        assert_eq!(padding_size(constants::MAX_PADDING), constants::MAX_PADDING);
        assert_eq!(
            padding_size(constants::MAX_PADDING + 1),
            constants::MAX_PADDING - 1
        );
        assert_eq!(
            padding_size(3 * constants::MAX_PADDING + 5),
            constants::MAX_PADDING - 5
        );
    }
}
