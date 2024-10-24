//! The `Id` type and related functions

use std::{fmt, io::Read, ops::Deref, path::Path, str::FromStr};

use binrw::{BinRead, BinWrite};
use derive_more::{Constructor, Display};
use rand::{thread_rng, RngCore};
use serde_derive::{Deserialize, Serialize};

use crate::{
    crypto::hasher::hash,
    error::{ErrorKind, RusticError, RusticResult},
};

pub(super) mod constants {
    /// The length of the hash in bytes
    pub(super) const LEN: usize = 32;
    /// The length of the hash in hexadecimal characters
    pub(super) const HEX_LEN: usize = LEN * 2;
}

#[macro_export]
/// Generate newtypes for `Id`s identifying Repository files
macro_rules! define_new_id_struct {
    ($a:ident, $b: expr) => {
        #[doc = concat!("An Id identifying a ", stringify!($b))]
        #[derive(
            Debug,
            Clone,
            Copy,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            derive_more::Deref,
            derive_more::Display,
            derive_more::From,
            derive_more::FromStr,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $a($crate::Id);

        impl $a {
            /// impl into_inner
            #[must_use]
            pub fn into_inner(self) -> $crate::Id {
                self.0
            }
        }
    };
}

/// `Id` is the hash id of an object.
///
/// It is being used to identify blobs or files saved in the repository.
#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Constructor,
    BinWrite,
    BinRead,
    Display,
)]
#[display("{}", &self.to_hex()[0..8])]
pub struct Id(
    /// The actual hash
    #[serde(serialize_with = "hex::serde::serialize")]
    #[serde(deserialize_with = "hex::serde::deserialize")]
    [u8; constants::LEN],
);

impl FromStr for Id {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut id = Self::default();
        hex::decode_to_slice(s, &mut id.0).map_err(|err| {
            RusticError::new(ErrorKind::Parsing,
                format!("Failed to decode hex string into Id. The string must be a valid hexadecimal string: {s}")
            ).source(err.into())
        })?;

        Ok(id)
    }
}

impl Id {
    /// Parse an `Id` from a hexadecimal string
    ///
    /// # Arguments
    ///
    /// * `s` - The hexadecimal string to parse
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    ///
    /// # Examples
    ///
    /// ```
    /// use rustic_core::Id;
    ///
    /// let id = Id::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    ///
    /// assert_eq!(id.to_hex().as_str(), "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    /// ```
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    #[deprecated(note = "use FromStr::from_str instead")]
    pub fn from_hex(s: &str) -> RusticResult<Self> {
        s.parse()
    }

    /// Generate a random `Id`.
    #[must_use]
    pub fn random_from_rng(rng: &mut impl RngCore) -> Self {
        let mut id = Self::default();
        rng.fill_bytes(&mut id.0);
        id
    }

    /// Generate a random `Id`.
    #[must_use]
    pub fn random() -> Self {
        Self::random_from_rng(&mut thread_rng())
    }

    /// Convert to [`HexId`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rustic_core::Id;
    ///
    /// let id = Id::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    ///
    /// assert_eq!(id.to_hex().as_str(), "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the `hex` crate fails to encode the hash
    // TODO! - remove the panic
    #[must_use]
    pub fn to_hex(self) -> HexId {
        let mut hex_id = HexId::EMPTY;
        // HexId's len is LEN * 2
        hex::encode_to_slice(self.0, &mut hex_id.0).unwrap();
        hex_id
    }

    /// Checks if the [`Id`] is zero
    ///
    /// # Examples
    ///
    /// ```
    /// use rustic_core::Id;
    ///
    /// let id = Id::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap();
    ///
    /// assert!(id.is_null());
    /// ```
    #[must_use]
    pub fn is_null(&self) -> bool {
        self == &Self::default()
    }

    /// Checks if this [`Id`] matches the content of a reader
    ///
    /// # Arguments
    ///
    /// * `length` - The length of the blob
    /// * `r` - The reader to check
    ///
    /// # Returns
    ///
    /// `true` if the SHA256 matches, `false` otherwise
    pub fn blob_matches_reader(&self, length: usize, r: &mut impl Read) -> bool {
        // check if SHA256 matches
        let mut vec = vec![0; length];
        r.read_exact(&mut vec).is_ok() && self == &hash(&vec)
    }

    /// returns the first 4 bytes as u32 (interpreted as little endian)
    #[must_use]
    pub fn as_u32(&self) -> u32 {
        u32::from_le_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }
}

impl fmt::Debug for Id {
    /// Format the `Id` as a hexadecimal string
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &*self.to_hex())
    }
}

/// An `Id` in hexadecimal format
#[derive(Copy, Clone, Debug)]
pub struct HexId([u8; constants::HEX_LEN]);

impl HexId {
    /// An empty [`HexId`]
    const EMPTY: Self = Self([b'0'; constants::HEX_LEN]);

    /// Get the string representation of a [`HexId`]
    ///
    /// # Panics
    ///
    /// If the [`HexId`] is not a valid UTF-8 string
    #[must_use]
    pub fn as_str(&self) -> &str {
        // This is only ever filled with hex chars, which are ascii
        std::str::from_utf8(&self.0).unwrap()
    }
}

impl Deref for HexId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<Path> for HexId {
    fn as_ref(&self) -> &Path {
        self.as_str().as_ref()
    }
}
