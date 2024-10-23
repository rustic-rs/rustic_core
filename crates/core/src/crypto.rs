pub(crate) mod aespoly1305;
pub(crate) mod hasher;

/// [`CryptoErrorKind`] describes the errors that can happen while dealing with Cryptographic functions
#[derive(thiserror::Error, Debug, displaydoc::Display, Copy, Clone)]
#[non_exhaustive]
pub enum CryptoErrorKind {
    /// data decryption failed: `{0:?}`
    DataDecryptionFailed(aes256ctr_poly1305aes::aead::Error),
    /// data encryption failed: `{0:?}`
    DataEncryptionFailed(aes256ctr_poly1305aes::aead::Error),
    /// crypto key too short
    CryptoKeyTooShort,
}

pub(crate) type CryptoResult<T> = Result<T, CryptoErrorKind>;

/// A trait for encrypting and decrypting data.
pub trait CryptoKey: Clone + Copy + Sized + Send + Sync + 'static {
    /// Decrypt the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to decrypt.
    ///
    /// # Returns
    ///
    /// A vector containing the decrypted data.
    fn decrypt_data(&self, data: &[u8]) -> CryptoResult<Vec<u8>>;

    /// Encrypt the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to encrypt.
    ///
    /// # Returns
    ///
    /// A vector containing the encrypted data.
    fn encrypt_data(&self, data: &[u8]) -> CryptoResult<Vec<u8>>;
}
