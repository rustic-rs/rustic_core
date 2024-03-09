use anyhow::Result;
use bytes::Bytes;
use zstd::decode_all;

use crate::{
    backend::{
        decrypt::{DecryptFullBackend, DecryptReadBackend, DecryptWriteBackend},
        FileType, ReadBackend, WriteBackend,
    },
    error::{CryptBackendErrorKind, RusticErrorKind, RusticResult},
    id::Id,
};

/// A backend implementation that does not actually write to the backend.
#[derive(Clone, Debug)]
pub struct DryRunBackend<BE: DecryptFullBackend> {
    /// The backend to use.
    be: BE,
    /// Whether to actually write to the backend.
    dry_run: bool,
}

impl<BE: DecryptFullBackend> DryRunBackend<BE> {
    /// Create a new [`DryRunBackend`].
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend to use.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use.
    /// * `dry_run` - Whether to actually write to the backend.
    pub const fn new(be: BE, dry_run: bool) -> Self {
        Self { be, dry_run }
    }
}

impl<BE: DecryptFullBackend> DecryptReadBackend for DryRunBackend<BE> {
    fn decrypt(&self, data: &[u8]) -> RusticResult<Vec<u8>> {
        self.be.decrypt(data)
    }

    /// Reads encrypted data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::DecryptionNotSupportedForBackend`] - If the backend does not support decryption.
    /// * [`CryptBackendErrorKind::DecodingZstdCompressedDataFailed`] - If decoding the zstd compressed data failed.
    ///
    /// # Returns
    ///
    /// The data read.
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
            _ => return Err(CryptBackendErrorKind::DecryptionNotSupportedForBackend.into()),
        }
        .into())
    }
}

impl<BE: DecryptFullBackend> ReadBackend for DryRunBackend<BE> {
    fn location(&self) -> String {
        self.be.location()
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

impl<BE: DecryptFullBackend> DecryptWriteBackend for DryRunBackend<BE> {
    type Key = <BE as DecryptWriteBackend>::Key;

    fn key(&self) -> &Self::Key {
        self.be.key()
    }

    fn hash_write_full(&self, tpe: FileType, data: &[u8]) -> RusticResult<Id> {
        if self.dry_run {
            Ok(Id::default())
        } else {
            self.be.hash_write_full(tpe, data)
        }
    }

    fn process_data(
        &self,
        data: &[u8],
    ) -> RusticResult<(Vec<u8>, u32, Option<std::num::NonZeroU32>)> {
        self.be.process_data(data)
    }

    fn set_zstd(&mut self, zstd: Option<i32>) {
        if !self.dry_run {
            self.be.set_zstd(zstd);
        }
    }

    fn set_extra_verify(&mut self, extra_check: bool) {
        if !self.dry_run {
            self.be.set_extra_verify(extra_check);
        }
    }
}

impl<BE: DecryptFullBackend> WriteBackend for DryRunBackend<BE> {
    fn create(&self) -> Result<()> {
        if self.dry_run {
            Ok(())
        } else {
            self.be.create()
        }
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        if self.dry_run {
            Ok(())
        } else {
            self.be.write_bytes(tpe, id, cacheable, buf)
        }
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        if self.dry_run {
            Ok(())
        } else {
            self.be.remove(tpe, id, cacheable)
        }
    }
}
