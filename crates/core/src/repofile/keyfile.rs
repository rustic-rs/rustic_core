use chrono::{DateTime, Local};
use rand::{thread_rng, RngCore};
use scrypt::Params;
use serde_derive::{Deserialize, Serialize};
use serde_with::{base64::Base64, serde_as, skip_serializing_none};

use crate::{
    backend::{FileType, ReadBackend},
    crypto::{aespoly1305::Key, CryptoErrorKind, CryptoKey},
    error::RusticResult,
    impl_repoid,
};

/// [`KeyFileErrorKind`] describes the errors that can be returned for `KeyFile`s
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum KeyFileErrorKind {
    /// no suitable key found!
    NoSuitableKeyFound,
    /// listing KeyFiles failed
    ListingKeyFilesFailed,
    /// couldn't get KeyFile from backend
    CouldNotGetKeyFileFromBackend,
    /// serde_json couldn't deserialize the data for the key: `{key_id:?}` : `{source}`
    DeserializingFromSliceForKeyIdFailed {
        /// The id of the key
        key_id: KeyId,
        /// The error that occurred
        source: serde_json::Error,
    },
    /// serde_json couldn't serialize the data into a JSON byte vector: `{0:?}`
    CouldNotSerializeAsJsonByteVector(serde_json::Error),
    /// output length is invalid: `{0:?}`
    OutputLengthInvalid(scrypt::errors::InvalidOutputLen),
    /// invalid scrypt parameters: `{0:?}`
    InvalidSCryptParameters(scrypt::errors::InvalidParams),
    /// Could not get key from decrypt data: `{key:?}` : `{source}`
    CouldNotGetKeyFromDecryptData { key: Key, source: CryptoErrorKind },
    /// deserializing master key from slice failed: `{source}`
    DeserializingMasterKeyFromSliceFailed { source: serde_json::Error },
    /// conversion from {from} to {to} failed for {x} : {source}
    ConversionFailed {
        from: &'static str,
        to: &'static str,
        x: u32,
        source: std::num::TryFromIntError,
    },
}

pub(crate) type KeyFileResult<T> = Result<T, KeyFileErrorKind>;

pub(super) mod constants {
    /// Returns the number of bits of the given type.
    pub(super) const fn num_bits<T>() -> usize {
        // Needed for MSRV 1.76
        #![allow(unused_qualifications)]
        std::mem::size_of::<T>() * 8
    }
}

impl_repoid!(KeyId, FileType::Key);

/// Key files describe information about repository access keys.
///
/// They are usually stored in the repository under `/keys/<ID>`
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug)]
pub struct KeyFile {
    /// Hostname where the key was created
    hostname: Option<String>,

    /// User which created the key
    username: Option<String>,

    /// Creation time of the key
    created: Option<DateTime<Local>>,

    /// The used key derivation function (currently only `scrypt`)
    kdf: String,

    /// Parameter N for `scrypt`
    #[serde(rename = "N")]
    n: u32,

    /// Parameter r for `scrypt`
    r: u32,

    /// Parameter p for `scrypt`
    p: u32,

    /// The key data encrypted by `scrypt`
    #[serde_as(as = "Base64")]
    data: Vec<u8>,

    /// The salt used with `scrypt`
    #[serde_as(as = "Base64")]
    salt: Vec<u8>,
}

impl KeyFile {
    /// Generate a Key using the key derivation function from [`KeyFile`] and a given password
    ///
    /// # Arguments
    ///
    /// * `passwd` - The password to use for the key derivation function
    ///
    /// # Errors
    ///
    /// * [`KeyFileErrorKind::InvalidSCryptParameters`] - If the parameters of the key derivation function are invalid
    /// * [`KeyFileErrorKind::OutputLengthInvalid`] - If the output length of the key derivation function is invalid
    ///
    /// # Returns
    ///
    /// The generated key
    ///
    /// [`KeyFileErrorKind::InvalidSCryptParameters`]: crate::error::KeyFileErrorKind::InvalidSCryptParameters
    /// [`KeyFileErrorKind::OutputLengthInvalid`]: crate::error::KeyFileErrorKind::OutputLengthInvalid
    pub fn kdf_key(&self, passwd: &impl AsRef<[u8]>) -> RusticResult<Key> {
        let params = Params::new(
            log_2(self.n).map_err(|_err| todo!("Error transition"))?,
            self.r,
            self.p,
            Params::RECOMMENDED_LEN,
        )
        .map_err(KeyFileErrorKind::InvalidSCryptParameters)
        .map_err(|_err| todo!("Error transition"))?;

        let mut key = [0; 64];
        scrypt::scrypt(passwd.as_ref(), &self.salt, &params, &mut key)
            .map_err(KeyFileErrorKind::OutputLengthInvalid)
            .map_err(|_err| todo!("Error transition"))?;

        Ok(Key::from_slice(&key))
    }

    /// Extract a key from the data of the [`KeyFile`] using the given key.
    /// The key usually should be the key generated by [`kdf_key()`](Self::kdf_key)
    ///
    /// # Arguments
    ///
    /// * `key` - The key to use for decryption
    ///
    /// # Errors
    ///
    /// * [`KeyFileErrorKind::DeserializingFromSliceFailed`] - If the data could not be deserialized
    ///
    /// # Returns
    ///
    /// The extracted key
    ///
    /// [`KeyFileErrorKind::DeserializingFromSliceFailed`]: crate::error::KeyFileErrorKind::DeserializingFromSliceFailed
    pub fn key_from_data(&self, key: &Key) -> RusticResult<Key> {
        let dec_data = key
            .decrypt_data(&self.data)
            .map_err(|err| KeyFileErrorKind::CouldNotGetKeyFromDecryptData {
                key: key.clone(),
                source: err,
            })
            .map_err(|_err| todo!("Error transition"))?;

        Ok(serde_json::from_slice::<MasterKey>(&dec_data)
            .map_err(|err| KeyFileErrorKind::DeserializingMasterKeyFromSliceFailed { source: err })
            .map_err(|_err| todo!("Error transition"))?
            .key())
    }

    /// Extract a key from the data of the [`KeyFile`] using the key
    /// from the derivation function in combination with the given password.
    ///
    /// # Arguments
    ///
    /// * `passwd` - The password to use for the key derivation function
    ///
    /// # Errors
    ///
    /// * [`KeyFileErrorKind::InvalidSCryptParameters`] - If the parameters of the key derivation function are invalid
    ///
    /// # Returns
    ///
    /// The extracted key
    ///
    /// [`KeyFileErrorKind::InvalidSCryptParameters`]: crate::error::KeyFileErrorKind::InvalidSCryptParameters
    pub fn key_from_password(&self, passwd: &impl AsRef<[u8]>) -> RusticResult<Key> {
        self.key_from_data(&self.kdf_key(passwd)?)
    }

    /// Generate a new [`KeyFile`] from a given key and password.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to use for encryption
    /// * `passwd` - The password to use for the key derivation function
    /// * `hostname` - The hostname to use for the [`KeyFile`]
    /// * `username` - The username to use for the [`KeyFile`]
    /// * `with_created` - Whether to set the creation time of the [`KeyFile`] to the current time
    ///
    /// # Errors
    ///
    /// * [`KeyFileErrorKind::OutputLengthInvalid`] - If the output length of the key derivation function is invalid
    /// * [`KeyFileErrorKind::CouldNotSerializeAsJsonByteVector`] - If the [`KeyFile`] could not be serialized
    ///
    /// # Returns
    ///
    /// The generated [`KeyFile`]
    ///
    /// [`KeyFileErrorKind::OutputLengthInvalid`]: crate::error::KeyFileErrorKind::OutputLengthInvalid
    /// [`KeyFileErrorKind::CouldNotSerializeAsJsonByteVector`]: crate::error::KeyFileErrorKind::CouldNotSerializeAsJsonByteVector
    pub fn generate(
        key: Key,
        passwd: &impl AsRef<[u8]>,
        hostname: Option<String>,
        username: Option<String>,
        with_created: bool,
    ) -> RusticResult<Self> {
        let masterkey = MasterKey::from_key(key);
        let params = Params::recommended();
        let mut salt = vec![0; 64];
        thread_rng().fill_bytes(&mut salt);

        let mut key = [0; 64];
        scrypt::scrypt(passwd.as_ref(), &salt, &params, &mut key)
            .map_err(KeyFileErrorKind::OutputLengthInvalid)
            .map_err(|_err| todo!("Error transition"))?;

        let key = Key::from_slice(&key);
        let data = key
            .encrypt_data(
                &serde_json::to_vec(&masterkey)
                    .map_err(KeyFileErrorKind::CouldNotSerializeAsJsonByteVector)
                    .map_err(|_err| todo!("Error transition"))?,
            )
            .map_err(|_err| todo!("Error transition"))?;

        Ok(Self {
            hostname,
            username,
            kdf: "scrypt".to_string(),
            n: 2_u32.pow(u32::from(params.log_n())),
            r: params.r(),
            p: params.p(),
            created: with_created.then(Local::now),
            data,
            salt,
        })
    }

    /// Get a [`KeyFile`] from the backend
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use
    /// * `id` - The id of the [`KeyFile`]
    ///
    /// # Errors
    ///
    // TODO!: Add errors!
    ///
    /// # Returns
    ///
    /// The [`KeyFile`] read from the backend
    fn from_backend<B: ReadBackend>(be: &B, id: &KeyId) -> RusticResult<Self> {
        let data = be.read_full(FileType::Key, id)?;

        Ok(serde_json::from_slice(&data)
            .map_err(
                |err| KeyFileErrorKind::DeserializingFromSliceForKeyIdFailed {
                    key_id: id.clone(),
                    source: err,
                },
            )
            .map_err(|_err| todo!("Error transition"))?)
    }
}

/// Calculate the logarithm to base 2 of the given number
///
/// # Arguments
///
/// * `x` - The number to calculate the logarithm to base 2 of
///
/// # Errors
///
/// * [`KeyFileErrorKind::ConversionFromU32ToU8Failed`] - If the conversion from `u32` to `u8` failed
///
/// # Returns
///
/// The logarithm to base 2 of the given number
///
/// [`KeyFileErrorKind::ConversionFromU32ToU8Failed`]: crate::error::KeyFileErrorKind::ConversionFromU32ToU8Failed
fn log_2(x: u32) -> KeyFileResult<u8> {
    assert!(x > 0);
    Ok(u8::try_from(constants::num_bits::<u32>()).map_err(|err| {
        KeyFileErrorKind::ConversionFailed {
            from: "usize",
            to: "u8",
            x,
            source: err,
        }
    })? - u8::try_from(x.leading_zeros()).map_err(|err| KeyFileErrorKind::ConversionFailed {
        from: "u32",
        to: "u8",
        x,
        source: err,
    })? - 1)
}

/// The mac of a [`Key`]
///
/// This is used to verify the integrity of the key
#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Mac {
    /// The key used for the mac
    #[serde_as(as = "Base64")]
    k: Vec<u8>,

    /// The random value used for the mac
    #[serde_as(as = "Base64")]
    r: Vec<u8>,
}

/// The master key of a [`Key`]
///
/// This is used to encrypt the key
#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct MasterKey {
    /// The mac of the key
    mac: Mac,
    /// The encrypted key
    #[serde_as(as = "Base64")]
    encrypt: Vec<u8>,
}

impl MasterKey {
    /// Create a [`MasterKey`] from a [`Key`]
    ///
    /// # Arguments
    ///
    /// * `key` - The key to create the [`MasterKey`] from
    ///
    /// # Returns
    ///
    /// The created [`MasterKey`]
    fn from_key(key: Key) -> Self {
        let (encrypt, k, r) = key.to_keys();
        Self {
            encrypt,
            mac: Mac { k, r },
        }
    }

    /// Get the [`Key`] from the [`MasterKey`]
    fn key(&self) -> Key {
        Key::from_keys(&self.encrypt, &self.mac.k, &self.mac.r)
    }
}

/// Get a [`KeyFile`] from the backend
///
/// # Arguments
///
/// * `be` - The backend to use
/// * `id` - The id of the [`KeyFile`]
/// * `passwd` - The password to use
///
/// # Errors
///
// TODO!: Add errors!
pub(crate) fn key_from_backend<B: ReadBackend>(
    be: &B,
    id: &KeyId,
    passwd: &impl AsRef<[u8]>,
) -> RusticResult<Key> {
    KeyFile::from_backend(be, id)?.key_from_password(passwd)
}

/// Find a [`KeyFile`] in the backend that fits to the given password and return the contained key.
/// If a key hint is given, only this key is tested.
/// This is recommended for a large number of keys.
///
/// # Arguments
///
/// * `be` - The backend to use
/// * `passwd` - The password to use
/// * `hint` - The key hint to use
///
/// # Errors
///
/// * [`KeyFileErrorKind::NoSuitableKeyFound`] - If no suitable key was found
///
/// # Returns
///
/// The found key
///
/// [`KeyFileErrorKind::NoSuitableKeyFound`]: crate::error::KeyFileErrorKind::NoSuitableKeyFound
pub(crate) fn find_key_in_backend<B: ReadBackend>(
    be: &B,
    passwd: &impl AsRef<[u8]>,
    hint: Option<&KeyId>,
) -> RusticResult<Key> {
    if let Some(id) = hint {
        key_from_backend(be, id, passwd)
    } else {
        for id in be.list(FileType::Key)? {
            match key_from_backend(be, &id.into(), passwd) {
                Ok(key) => return Ok(key),
                // TODO: We get a RusticError here and we need to determine, if we have a WrongKey error
                // TODO: We should probably implement something for that on RusticError or use a variant for this
                Err(KeyFileErrorKind::DataDecryptionFailed(_)) => continue,
                err => return err,
            }
        }
        Err(KeyFileErrorKind::NoSuitableKeyFound).map_err(|_err| todo!("Error transition"))
    }
}
