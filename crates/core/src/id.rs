//! The `Id` type and related functions

use std::{
    fmt::{self, Display},
    io::Read,
    ops::Deref,
    path::Path,
    str::FromStr,
};

use binrw::{BinRead, BinWrite};
use derive_more::Constructor;
use rand::{thread_rng, RngCore};
use serde_derive::{Deserialize, Serialize};

use crate::{crypto::hasher::hash, error::IdErrorKind, RusticResult};

pub(super) mod constants {
    /// The length of the hash in bytes
    pub(super) const LEN: usize = 32;
    /// The length of the hash in hexadecimal characters
    pub(super) const HEX_LEN: usize = LEN * 2;
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
)]
pub struct Id(
    /// The actual hash
    #[serde(serialize_with = "hex::serde::serialize")]
    #[serde(deserialize_with = "hex::serde::deserialize")]
    [u8; constants::LEN],
);

impl FromStr for Id {
    type Err = IdErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s).map_err(|_| IdErrorKind::ParsingIdFromStringFailed(s.to_string()))
    }
}

impl Display for Id {
    /// Format the `Id` as a hexadecimal string
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = &self.to_hex()[0..8];

        write!(f, "{id}")
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
    /// assert_eq!(id.to_hex().as_str(),
    /// "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    /// ```
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    pub fn from_hex(s: &str) -> RusticResult<Self> {
        if s.is_empty() {
            return Err(IdErrorKind::EmptyHexString.into());
        }

        if !s.is_ascii() {
            return Err(IdErrorKind::NonAsciiHexString.into());
        }

        let mut id = Self::default();

        hex::decode_to_slice(s, &mut id.0).map_err(IdErrorKind::HexError)?;

        Ok(id)
    }

    /// Generate a random `Id`.
    #[must_use]
    pub fn random() -> Self {
        let mut id = Self::default();
        thread_rng().fill_bytes(&mut id.0);
        id
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
    /// - An invalid character was found. Valid ones are: `0...9`, `a...f` or `A...F`.
    ///
    /// - A hex string's length needs to be even, as two digits correspond to one byte.
    ///
    /// - If the hex string is decoded into a fixed sized container, such as an
    ///   array, the hex string's length * 2 has to match the container's length.
    ///
    /// # Returns
    ///
    /// The [`HexId`] representation of the [`Id`] if it is valid
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn to_hex(self) -> HexId {
        let mut hex_id = HexId::EMPTY;

        hex::encode_to_slice(self.0, &mut hex_id.0)
            .expect("HexId's len is LEN * 2, should never panic.");

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
}

impl fmt::Debug for Id {
    /// Format the `Id` as a hexadecimal string
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = &self.to_hex()[0..32];

        write!(f, "{id}")
    }
}

/// An `Id` in hexadecimal format
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HexId([u8; constants::HEX_LEN]);

impl From<Id> for HexId {
    fn from(id: Id) -> Self {
        id.to_hex()
    }
}

impl PartialEq<str> for HexId {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl HexId {
    /// An empty [`HexId`]
    const EMPTY: Self = Self([b'0'; constants::HEX_LEN]);

    /// Get the string representation of a [`HexId`]
    ///
    /// # Panics
    ///
    /// * `None` case: the end of the input was reached unexpectedly.
    ///   `self.valid_up_to()` is 1 to 3 bytes from the end of the input.
    ///   If a byte stream (such as a file or a network socket) is being decoded incrementally,
    ///   this could be a valid `char` whose UTF-8 byte sequence is spanning multiple chunks.
    ///
    /// * `Some(len)` case: an unexpected byte was encountered.
    ///   The length provided is that of the invalid byte sequence
    ///   that starts at the index given by `valid_up_to()`.
    ///   Decoding should resume after that sequence
    ///   (after inserting a [`U+FFFD REPLACEMENT CHARACTER`][U+FFFD]) in case of
    ///   lossy decoding.
    ///
    /// # Returns
    ///
    /// The string representation of the [`HexId`] if it is valid
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn as_str(&self) -> &str {
        // This is only ever filled with hex chars, which are ascii
        std::str::from_utf8(&self.0).expect("HexId is not valid utf8, which should never happen")
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {

    use rstest::rstest;
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn test_id_to_hex_to_str_fails() {
        let non_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdefZ";
        let id = non_hex.parse::<Id>();

        assert!(id.is_err(), "Id with non-hex str passed");
    }

    #[test]
    fn test_id_is_random_passes() {
        let mut ids = vec![Id::default(); 100_000];

        for id in &mut ids {
            *id = Id::random();
        }

        let set = ids.iter().collect::<std::collections::HashSet<_>>();

        assert_eq!(set.len(), ids.len(), "Random ids are not unique");

        for id in ids {
            assert!(!id.is_null(), "Random id is null");
        }
    }

    #[test]
    fn test_id_blob_matches_reader_passes() {
        let id_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let id = Id::new(Sha256::digest(id_str).into());

        let mut reader = std::io::Cursor::new(id_str);

        let length = 64;

        assert!(
            id.blob_matches_reader(length, &mut reader),
            "Blob does not match reader"
        );
    }

    #[test]
    fn test_id_is_null_passes() {
        let id = "".parse::<Id>();

        assert!(id.is_err(), "Empty id is not null");
    }

    #[rstest]
    #[case("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")]
    fn test_parse_id_from_str_passes(#[case] id_str: &str) {
        let id = id_str.parse::<Id>();

        assert!(id.is_ok(), "Id parsing failed");

        let id = id.unwrap().to_hex();

        assert_eq!(id.as_str(), id_str, "Id to hex to str failed");
    }

    #[test]
    fn test_from_id_to_hex_passes() {
        let id_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let id = Id::from_hex(id_str).unwrap();

        let hex_id = HexId::from(id);

        assert_eq!(hex_id.as_str(), id_str, "Id to hex to str failed");
    }
}
