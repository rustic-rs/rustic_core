use std::{num::NonZeroU32, sync::Arc};

use anyhow::Result;
use bytes::Bytes;
use crossbeam_channel::{unbounded, Receiver};
use rayon::prelude::*;
use zstd::stream::{copy_encode, decode_all};

pub use zstd::compression_level_range;

/// The maximum compression level allowed by zstd
#[must_use]
pub fn max_compression_level() -> i32 {
    *compression_level_range().end()
}

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    crypto::{hasher::hash, CryptoKey},
    error::{CryptBackendErrorKind, RusticErrorKind},
    id::Id,
    repofile::RepoFile,
    Progress, RusticResult,
};

/// A backend that can decrypt data.
/// This is a trait that is implemented by all backends that can decrypt data.
/// It is implemented for all backends that implement `DecryptWriteBackend` and `DecryptReadBackend`.
/// This trait is used by the `Repository` to decrypt data.
pub trait DecryptFullBackend: DecryptWriteBackend + DecryptReadBackend {}

impl<T: DecryptWriteBackend + DecryptReadBackend> DecryptFullBackend for T {}

pub trait DecryptReadBackend: ReadBackend + Clone + 'static {
    /// Decrypts the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to decrypt.
    ///
    /// # Errors
    ///
    /// If the data could not be decrypted.
    fn decrypt(&self, data: &[u8]) -> RusticResult<Vec<u8>>;

    /// Reads the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// If the file could not be read.
    fn read_encrypted_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes>;

    /// Reads the given file from partial data.
    ///
    /// # Arguments
    ///
    /// * `data` - The partial data to decrypt.
    /// * `uncompressed_length` - The length of the uncompressed data.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::DecodingZstdCompressedDataFailed`] - If the data could not be decoded.
    /// * [`CryptBackendErrorKind::LengthOfUncompressedDataDoesNotMatch`] - If the length of the uncompressed data does not match the given length.
    ///
    /// [`CryptBackendErrorKind::DecodingZstdCompressedDataFailed`]: crate::error::CryptBackendErrorKind::DecodingZstdCompressedDataFailed
    /// [`CryptBackendErrorKind::LengthOfUncompressedDataDoesNotMatch`]: crate::error::CryptBackendErrorKind::LengthOfUncompressedDataDoesNotMatch
    fn read_encrypted_from_partial(
        &self,
        data: &[u8],
        uncompressed_length: Option<NonZeroU32>,
    ) -> RusticResult<Bytes> {
        let mut data = self.decrypt(data)?;
        if let Some(length) = uncompressed_length {
            data = decode_all(&*data)
                .map_err(CryptBackendErrorKind::DecodingZstdCompressedDataFailed)?;
            if data.len() != length.get() as usize {
                return Err(CryptBackendErrorKind::LengthOfUncompressedDataDoesNotMatch.into());
            }
        }
        Ok(data.into())
    }

    /// Reads the given file with the given offset and length.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file should be cached.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    /// * `uncompressed_length` - The length of the uncompressed data.
    ///
    /// # Errors
    ///
    /// If the file could not be read.
    fn read_encrypted_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
        uncompressed_length: Option<NonZeroU32>,
    ) -> RusticResult<Bytes> {
        self.read_encrypted_from_partial(
            &self
                .read_partial(tpe, id, cacheable, offset, length)
                .map_err(RusticErrorKind::Backend)?,
            uncompressed_length,
        )
    }

    /// Gets the given file.
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// If the file could not be read.
    fn get_file<F: RepoFile>(&self, id: &Id) -> RusticResult<F> {
        let data = self.read_encrypted_full(F::TYPE, id)?;
        Ok(serde_json::from_slice(&data)
            .map_err(CryptBackendErrorKind::DeserializingFromBytesOfJsonTextFailed)?)
    }

    /// Streams all files.
    ///
    /// # Arguments
    ///
    /// * `p` - The progress bar.
    ///
    /// # Errors
    ///
    /// If the files could not be read.
    fn stream_all<F: RepoFile>(
        &self,
        p: &impl Progress,
    ) -> RusticResult<Receiver<RusticResult<(Id, F)>>> {
        let list = self.list(F::TYPE).map_err(RusticErrorKind::Backend)?;
        self.stream_list(list, p)
    }

    /// Streams a list of files.
    ///
    /// # Arguments
    ///
    /// * `list` - The list of files to stream.
    /// * `p` - The progress bar.
    ///
    /// # Errors
    ///
    /// If the files could not be read.
    fn stream_list<F: RepoFile>(
        &self,
        list: Vec<Id>,
        p: &impl Progress,
    ) -> RusticResult<Receiver<RusticResult<(Id, F)>>> {
        p.set_length(list.len() as u64);
        let (tx, rx) = unbounded();

        list.into_par_iter()
            .for_each_with((self, p, tx), |(be, p, tx), id| {
                let file = be.get_file::<F>(&id).map(|file| (id, file));
                p.inc(1);
                tx.send(file).unwrap();
            });
        Ok(rx)
    }
}

pub trait DecryptWriteBackend: WriteBackend + Clone + 'static {
    /// The type of the key.
    type Key: CryptoKey;

    /// Gets the key.
    fn key(&self) -> &Self::Key;

    /// Writes the given data to the backend and returns the id of the data.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `data` - The data to write.
    ///
    /// # Errors
    ///
    /// If the data could not be written.
    ///
    /// # Returns
    ///
    /// The hash of the written data.
    fn hash_write_full(&self, tpe: FileType, data: &[u8]) -> RusticResult<Id>;

    /// Writes the given data to the backend without compression and returns the id of the data.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `data` - The data to write.
    ///
    /// # Errors
    ///
    /// If the data could not be written.
    ///
    /// # Returns
    ///
    /// The hash of the written data.
    fn hash_write_full_uncompressed(&self, tpe: FileType, data: &[u8]) -> RusticResult<Id> {
        let data = self.key().encrypt_data(data)?;
        let id = hash(&data);
        self.write_bytes(tpe, &id, false, data.into())
            .map_err(RusticErrorKind::Backend)?;
        Ok(id)
    }
    /// Saves the given file.
    ///
    /// # Arguments
    ///
    /// * `file` - The file to save.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
    ///
    /// # Returns
    ///
    /// The id of the file.
    ///
    /// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
    fn save_file<F: RepoFile>(&self, file: &F) -> RusticResult<Id> {
        let data = serde_json::to_vec(file)
            .map_err(CryptBackendErrorKind::SerializingToJsonByteVectorFailed)?;
        self.hash_write_full(F::TYPE, &data)
    }

    /// Saves the given file uncompressed.
    ///
    /// # Arguments
    ///
    /// * `file` - The file to save.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
    ///
    /// # Returns
    ///
    /// The id of the file.
    ///
    /// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
    fn save_file_uncompressed<F: RepoFile>(&self, file: &F) -> RusticResult<Id> {
        let data = serde_json::to_vec(file)
            .map_err(CryptBackendErrorKind::SerializingToJsonByteVectorFailed)?;
        self.hash_write_full_uncompressed(F::TYPE, &data)
    }

    /// Saves the given list of files.
    ///
    /// # Arguments
    ///
    /// * `list` - The list of files to save.
    /// * `p` - The progress bar.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
    fn save_list<'a, F: RepoFile, I: ExactSizeIterator<Item = &'a F> + Send>(
        &self,
        list: I,
        p: impl Progress,
    ) -> RusticResult<()> {
        p.set_length(list.len() as u64);
        list.par_bridge().try_for_each(|file| -> RusticResult<_> {
            _ = self.save_file(file)?;
            p.inc(1);
            Ok(())
        })?;
        p.finish();
        Ok(())
    }

    /// Deletes the given list of files.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files.
    /// * `cacheable` - Whether the files should be cached.
    /// * `list` - The list of files to delete.
    /// * `p` - The progress bar.
    ///
    /// # Panics
    ///
    /// If the files could not be deleted.
    fn delete_list<'a, I: ExactSizeIterator<Item = &'a Id> + Send>(
        &self,
        tpe: FileType,
        cacheable: bool,
        list: I,
        p: impl Progress,
    ) -> RusticResult<()> {
        p.set_length(list.len() as u64);
        list.par_bridge().try_for_each(|id| -> RusticResult<_> {
            // TODO: Don't panic on file not being able to be deleted.
            self.remove(tpe, id, cacheable).unwrap();
            p.inc(1);
            Ok(())
        })?;

        p.finish();
        Ok(())
    }

    fn set_zstd(&mut self, zstd: Option<i32>);
}

/// A backend that can decrypt data.
///
/// # Type Parameters
///
/// * `C` - The type of the key to decrypt the backend with.
#[derive(Debug, Clone)]
pub struct DecryptBackend<C: CryptoKey> {
    /// The backend to decrypt.
    be: Arc<dyn WriteBackend>,
    /// The key to decrypt the backend with.
    key: C,
    /// The compression level to use for zstd.
    zstd: Option<i32>,
}

impl<C: CryptoKey> DecryptBackend<C> {
    /// Creates a new decrypt backend.
    ///
    /// # Type Parameters
    ///
    /// * `C` - The type of the key to decrypt the backend with.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to decrypt.
    /// * `key` - The key to decrypt the backend with.
    ///
    /// # Returns
    ///
    /// The new decrypt backend.
    pub fn new(be: Arc<dyn WriteBackend>, key: C) -> Self {
        Self {
            be,
            key,
            zstd: None,
        }
    }
}

impl<C: CryptoKey> DecryptWriteBackend for DecryptBackend<C> {
    /// The type of the key.
    type Key = C;

    /// Gets the key.
    fn key(&self) -> &Self::Key {
        &self.key
    }

    /// Writes the given data to the backend and returns the id of the data.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `data` - The data to write.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::CopyEncodingDataFailed`] - If the data could not be encoded.
    ///
    /// # Returns
    ///
    /// The id of the data.
    ///
    /// [`CryptBackendErrorKind::CopyEncodingDataFailed`]: crate::error::CryptBackendErrorKind::CopyEncodingDataFailed
    fn hash_write_full(&self, tpe: FileType, data: &[u8]) -> RusticResult<Id> {
        let data = match self.zstd {
            Some(level) => {
                let mut out = vec![2_u8];
                copy_encode(data, &mut out, level)
                    .map_err(CryptBackendErrorKind::CopyEncodingDataFailed)?;
                self.key().encrypt_data(&out)?
            }
            None => self.key().encrypt_data(data)?,
        };
        let id = hash(&data);
        self.write_bytes(tpe, &id, false, data.into())
            .map_err(RusticErrorKind::Backend)?;
        Ok(id)
    }

    /// Sets the compression level to use for zstd.
    ///
    /// # Arguments
    ///
    /// * `zstd` - The compression level to use for zstd.
    fn set_zstd(&mut self, zstd: Option<i32>) {
        self.zstd = zstd;
    }
}

impl<C: CryptoKey> DecryptReadBackend for DecryptBackend<C> {
    /// Decrypts the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to decrypt.
    ///
    /// # Returns
    ///
    /// A vector containing the decrypted data.
    fn decrypt(&self, data: &[u8]) -> RusticResult<Vec<u8>> {
        self.key.decrypt_data(data)
    }

    /// Reads encrypted data from the backend.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::DecryptionNotSupportedForBackend`] - If the backend does not support decryption.
    /// * [`CryptBackendErrorKind::DecodingZstdCompressedDataFailed`] - If the data could not be decoded.
    ///
    /// [`CryptBackendErrorKind::DecryptionNotSupportedForBackend`]: crate::error::CryptBackendErrorKind::DecryptionNotSupportedForBackend
    /// [`CryptBackendErrorKind::DecodingZstdCompressedDataFailed`]: crate::error::CryptBackendErrorKind::DecodingZstdCompressedDataFailed
    fn read_encrypted_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        let decrypted =
            self.decrypt(&self.read_full(tpe, id).map_err(RusticErrorKind::Backend)?)?;
        Ok(match decrypted.first() {
            Some(b'{' | b'[') => decrypted, // not compressed
            Some(2) => decode_all(&decrypted[1..])
                .map_err(CryptBackendErrorKind::DecodingZstdCompressedDataFailed)?, // 2 indicates compressed data following
            _ => return Err(CryptBackendErrorKind::DecryptionNotSupportedForBackend)?,
        }
        .into())
    }
}

impl<C: CryptoKey> ReadBackend for DecryptBackend<C> {
    fn location(&self) -> String {
        self.be.location()
    }

    fn list(&self, tpe: FileType) -> Result<Vec<Id>> {
        self.be.list(tpe)
    }

    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.be.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        self.be.read_partial(tpe, id, cacheable, offset, length)
    }
}

impl<C: CryptoKey> WriteBackend for DecryptBackend<C> {
    fn create(&self) -> Result<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        self.be.remove(tpe, id, cacheable)
    }
}
