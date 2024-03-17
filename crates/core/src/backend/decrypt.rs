use std::{num::NonZeroU32, sync::Arc};

use anyhow::Result;
use bytes::Bytes;
use crossbeam_channel::{unbounded, Receiver};
use log::error;
use rayon::prelude::*;
use zstd::stream::{copy_encode, decode_all, encode_all};

pub use zstd::compression_level_range;

/// The maximum compression level allowed by zstd
#[must_use]
pub fn max_compression_level() -> i32 {
    *compression_level_range().end()
}

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    crypto::{hasher::hash, CryptoKey},
    error::{CryptBackendErrorKind, MultiprocessingErrorKind, RusticErrorKind},
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

        list.into_par_iter().try_for_each_with(
            (self, p, tx),
            |(be, p, tx), id| -> RusticResult<()> {
                let file = be.get_file::<F>(&id).map(|file| (id, file));
                p.inc(1);
                if tx.send(file).is_err() {
                    error!("receiver has been dropped unexpectedly.");
                    return Err(MultiprocessingErrorKind::ReceiverDropped.into());
                }
                Ok(())
            },
        )?;
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

    /// Process some blob data.
    /// This compresses and encrypts the data as requested
    ///
    /// # Returns
    ///
    /// The processed data, the original data length and when compression is used, the uncomressed length
    fn process_data(&self, data: &[u8]) -> RusticResult<(Vec<u8>, u32, Option<NonZeroU32>)>;

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
    /// # Errors
    ///
    /// If the files could not be deleted.
    ///
    /// # Returns
    ///
    /// Ok if the files were deleted.
    fn delete_list<'a, I: ExactSizeIterator<Item = &'a Id> + Send>(
        &self,
        tpe: FileType,
        cacheable: bool,
        list: I,
        p: impl Progress,
    ) -> RusticResult<()> {
        p.set_length(list.len() as u64);
        list.par_bridge().try_for_each(|id| -> RusticResult<_> {
            self.remove(tpe, id, cacheable)
                .map_err(RusticErrorKind::Backend)?;
            p.inc(1);
            Ok(())
        })?;

        p.finish();
        Ok(())
    }

    /// Sets the compression level to use for zstd.
    ///
    /// # Arguments
    ///
    /// * `zstd` - The compression level to use for zstd. TODO: What happens if this is None? What are defaults?
    fn set_zstd(&mut self, zstd: Option<i32>);
    fn set_extra_verify(&mut self, extra_check: bool);
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
    /// Whether to do an extra verification by decompressing and decrypting the data
    extra_verify: bool,
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
            // zstd and extra_verify are directly set, where needed.
            zstd: None,
            extra_verify: false,
        }
    }

    /// Decrypt and potentially decompress an already read repository file
    fn decrypt_file(&self, data: &[u8]) -> RusticResult<Vec<u8>> {
        let decrypted = self.decrypt(data)?;
        Ok(match decrypted.first() {
            Some(b'{' | b'[') => decrypted, // not compressed
            Some(2) => decode_all(&decrypted[1..])
                .map_err(CryptBackendErrorKind::DecodingZstdCompressedDataFailed)?, // 2 indicates compressed data following
            _ => return Err(CryptBackendErrorKind::DecryptionNotSupportedForBackend)?,
        })
    }

    /// encrypt and potentially compress a repository file
    fn encrypt_file(&self, data: &[u8]) -> RusticResult<Vec<u8>> {
        let data_encrypted = match self.zstd {
            Some(level) => {
                let mut out = vec![2_u8];
                copy_encode(data, &mut out, level)
                    .map_err(CryptBackendErrorKind::CopyEncodingDataFailed)?;
                self.key().encrypt_data(&out)?
            }
            None => self.key().encrypt_data(data)?,
        };
        Ok(data_encrypted)
    }

    fn very_file(&self, data_encrypted: &[u8], data: &[u8]) -> RusticResult<()> {
        if self.extra_verify {
            let check_data = self.decrypt_file(data_encrypted)?;
            if data != check_data {
                return Err(CryptBackendErrorKind::ExtraVerificationFailed.into());
            }
        }
        Ok(())
    }

    /// encrypt and potentially compress some data
    fn encrypt_data(&self, data: &[u8]) -> RusticResult<(Vec<u8>, u32, Option<NonZeroU32>)> {
        let data_len: u32 = data
            .len()
            .try_into()
            .map_err(CryptBackendErrorKind::IntConversionFailed)?;
        let (data_encrypted, uncompressed_length) = match self.zstd {
            None => (self.key.encrypt_data(data)?, None),
            // compress if requested
            Some(level) => (
                self.key.encrypt_data(&encode_all(data, level)?)?,
                NonZeroU32::new(data_len),
            ),
        };
        Ok((data_encrypted, data_len, uncompressed_length))
    }

    fn very_data(
        &self,
        data_encrypted: &[u8],
        uncompressed_length: Option<NonZeroU32>,
        data: &[u8],
    ) -> RusticResult<()> {
        if self.extra_verify {
            let data_check =
                self.read_encrypted_from_partial(data_encrypted, uncompressed_length)?;
            if data != data_check {
                return Err(CryptBackendErrorKind::ExtraVerificationFailed.into());
            }
        }
        Ok(())
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
        let data_encrypted = self.encrypt_file(data)?;
        self.very_file(&data_encrypted, data)?;
        let id = hash(&data_encrypted);
        self.write_bytes(tpe, &id, false, data_encrypted.into())
            .map_err(RusticErrorKind::Backend)?;
        Ok(id)
    }

    fn process_data(&self, data: &[u8]) -> RusticResult<(Vec<u8>, u32, Option<NonZeroU32>)> {
        let (data_encrypted, data_len, uncompressed_length) = self.encrypt_data(data)?;
        self.very_data(&data_encrypted, uncompressed_length, data)?;
        Ok((data_encrypted, data_len, uncompressed_length))
    }

    /// Sets the compression level to use for zstd.
    ///
    /// # Arguments
    ///
    /// * `zstd` - The compression level to use for zstd.
    fn set_zstd(&mut self, zstd: Option<i32>) {
        self.zstd = zstd;
    }

    /// Sets `extra_check`, i.e. whether to do an extra check after compressing/encrypting
    ///
    /// # Arguments
    ///
    /// * `extra_echeck` - The compression level to use for zstd.
    fn set_extra_verify(&mut self, extra_verify: bool) {
        self.extra_verify = extra_verify;
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
        self.decrypt_file(&self.read_full(tpe, id).map_err(RusticErrorKind::Backend)?)
            .map(Into::into)
    }
}

impl<C: CryptoKey> ReadBackend for DecryptBackend<C> {
    fn location(&self) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use crate::{backend::MockBackend, crypto::aespoly1305::Key};
    use anyhow::Result;

    use super::*;

    fn init() -> (DecryptBackend<Key>, &'static [u8]) {
        let be = Arc::new(MockBackend::new());
        let key = Key::new();
        let mut be = DecryptBackend::new(be, key);
        be.set_zstd(Some(0));
        let data = b"{test}"; // Note we use braces as this should be detected as valid json
        (be, data)
    }

    #[test]
    fn test_verify_encrypt_file_passes() -> Result<()> {
        let (mut be, data) = init();
        be.set_extra_verify(true);
        let data_encrypted = be.encrypt_file(data)?;
        be.very_file(&data_encrypted, data)?;
        Ok(())
    }

    #[test]
    fn test_verify_encrypt_file_no_test_passes() -> Result<()> {
        let (be, data) = init();
        let mut data_encrypted = be.encrypt_file(data)?;
        // modify some data
        data_encrypted[5] = !data_encrypted[5];
        // won't be detected
        be.very_file(&data_encrypted, data)?;
        Ok(())
    }

    #[test]
    fn test_verify_encrypt_file_nok_passes() -> Result<()> {
        let (mut be, data) = init();
        be.set_extra_verify(true);
        let mut data_encrypted = be.encrypt_file(data)?;
        // modify some data
        data_encrypted[5] = !data_encrypted[5];
        // will be detected
        assert!(be.very_file(&data_encrypted, data).is_err());
        Ok(())
    }

    #[test]
    fn test_verify_encrypt_data_passes() -> Result<()> {
        let (mut be, data) = init();
        be.set_extra_verify(true);
        let (data_encrypted, _, ul) = be.encrypt_data(data)?;
        be.very_data(&data_encrypted, ul, data)?;
        Ok(())
    }

    #[test]
    fn test_verify_encrypt_data_no_test_passes() -> Result<()> {
        let (be, data) = init();
        let (mut data_encrypted, _, ul) = be.encrypt_data(data)?;
        // modify some data
        data_encrypted[0] = !data_encrypted[0];
        // won't be detected
        be.very_data(&data_encrypted, ul, data)?;
        Ok(())
    }

    #[test]
    fn test_verify_encrypt_data_nok_passes() -> Result<()> {
        let (mut be, data) = init();
        be.set_extra_verify(true);
        let (mut data_encrypted, _, ul) = be.encrypt_data(data)?;
        // modify some data
        data_encrypted[5] = !data_encrypted[5];
        // will be detected
        assert!(be.very_data(&data_encrypted, ul, data).is_err());
        Ok(())
    }
}
