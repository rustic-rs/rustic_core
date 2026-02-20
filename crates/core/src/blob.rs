pub(crate) mod packer;
pub(crate) mod tree;

use std::{cmp::Ordering, num::NonZeroU32};

use derive_more::Constructor;
use enum_map::{Enum, EnumMap};
use serde_derive::{Deserialize, Serialize};

use crate::define_new_id_struct;

pub(super) mod constants {
    /// The maximum size of pack-part which is read at once from the backend.
    /// (needed to limit the memory size used for large backends)
    pub(crate) const LIMIT_PACK_READ: u32 = 40 * 1024 * 1024; // 40 MiB
    /// The maximum size of holes which are still read when repacking
    pub(crate) const MAX_HOLESIZE: u32 = 256 * 1024; // 256 kiB
}

/// All [`BlobType`]s which are supported by the repository
pub const ALL_BLOB_TYPES: [BlobType; 2] = [BlobType::Tree, BlobType::Data];

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Enum,
    derive_more::Display,
)]
/// The type a `blob` or a `packfile` can have
pub enum BlobType {
    #[serde(rename = "tree")]
    /// This is a tree blob
    Tree,
    #[serde(rename = "data")]
    /// This is a data blob
    Data,
}

impl BlobType {
    /// Defines the cacheability of a [`BlobType`]
    ///
    /// # Returns
    ///
    /// `true` if the [`BlobType`] is cacheable, `false` otherwise
    #[must_use]
    pub(crate) const fn is_cacheable(self) -> bool {
        match self {
            Self::Tree => true,
            Self::Data => false,
        }
    }
}

pub type BlobTypeMap<T> = EnumMap<BlobType, T>;

/// Initialize is a new trait to define the method `init()` for a [`BlobTypeMap`]
pub trait Initialize<T: Default + Sized> {
    /// Initialize a [`BlobTypeMap`] by processing a given function for each [`BlobType`]
    fn init<F: FnMut(BlobType) -> T>(init: F) -> BlobTypeMap<T>;
}

impl<T: Default> Initialize<T> for BlobTypeMap<T> {
    /// Initialize a [`BlobTypeMap`] by processing a given function for each [`BlobType`]
    ///
    /// # Arguments
    ///
    /// * `init` - The function to process for each [`BlobType`]
    ///
    /// # Returns
    ///
    /// A [`BlobTypeMap`] with the result of the function for each [`BlobType`]
    fn init<F: FnMut(BlobType) -> T>(mut init: F) -> Self {
        let mut btm = Self::default();
        for i in 0..BlobType::LENGTH {
            let bt = BlobType::from_usize(i);
            btm[bt] = init(bt);
        }
        btm
    }
}

define_new_id_struct!(BlobId, "blob");

/// A marker trait for Ids which identify Blobs in pack files
pub trait PackedId: Copy + Into<BlobId> + From<BlobId> {
    /// The `BlobType` of the blob identified by the Id
    const TYPE: BlobType;
}

#[macro_export]
/// Generate newtypes for `Id`s identifying packed blobs
macro_rules! impl_blobid {
    ($a:ident, $b: expr) => {
        $crate::define_new_id_struct!($a, concat!("blob of type", stringify!($b)));
        impl From<$crate::blob::BlobId> for $a {
            fn from(id: $crate::blob::BlobId) -> Self {
                (*id).into()
            }
        }
        impl From<$a> for $crate::blob::BlobId {
            fn from(id: $a) -> Self {
                (*id).into()
            }
        }
        impl $crate::blob::PackedId for $a {
            const TYPE: $crate::blob::BlobType = $b;
        }
    };
}

impl_blobid!(DataId, BlobType::Data);

/// A `Blob` is a file that is stored in the backend.
///
/// It can be a `tree` or a `data` blob.
///
/// A `tree` blob is a file that contains a list of other blobs.
/// A `data` blob is a file that contains the actual data.
#[derive(Debug, PartialEq, Eq, Copy, Clone, Constructor)]
pub(crate) struct Blob {
    /// The type of the blob
    tpe: BlobType,

    /// The id of the blob
    id: BlobId,
}

/// `BlobLocation` contains information about a blob within a pack
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobLocation {
    /// The offset of the blob within the pack
    pub offset: u32,
    /// The length of the blob
    pub length: u32,
    /// The uncompressed length of the blob
    pub uncompressed_length: Option<NonZeroU32>,
}

impl PartialOrd<Self> for BlobLocation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BlobLocation {
    fn cmp(&self, other: &Self) -> Ordering {
        self.offset.cmp(&other.offset)
    }
}

impl BlobLocation {
    /// Get the length of the data contained in this blob
    pub const fn data_length(&self) -> u32 {
        match self.uncompressed_length {
            None => self.length - 32,
            Some(length) => NonZeroU32::get(length),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct BlobLocations<T> {
    pub offset: u32,
    pub length: u32,
    pub blobs: Vec<(BlobLocation, T)>,
}

impl<T: Eq + PartialEq> PartialOrd<Self> for BlobLocations<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Eq> Ord for BlobLocations<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.offset.cmp(&other.offset)
    }
}

impl<T> BlobLocations<T> {
    pub fn length(&self) -> u32 {
        self.blobs.iter().map(|bl| bl.0.length).sum()
    }

    pub fn data_length(&self) -> u32 {
        self.blobs.iter().map(|bl| bl.0.data_length()).sum()
    }

    pub fn from_blob_location(location: BlobLocation, target: T) -> Self {
        Self {
            offset: location.offset,
            length: location.length,
            blobs: vec![(location, target)],
        }
    }
    pub fn can_coalesce(&self, other: &Self) -> bool {
        // if the blobs are (almost) contiguous and we don't trespass the limit, blobs can be read in one partial read
        other.offset <= self.offset + self.length + constants::MAX_HOLESIZE
            && other.offset >= self.offset + self.length
            && other.offset + other.length - self.offset <= constants::LIMIT_PACK_READ
    }

    pub fn append(mut self, mut other: Self) -> Self {
        self.length = other.offset + other.length - self.offset; // read till the end of other
        self.blobs.append(&mut other.blobs);
        self
    }

    #[allow(clippy::result_large_err)]
    /// coalesce two `BlobLocations` if possible
    pub fn coalesce(self, other: Self) -> Result<Self, (Self, Self)> {
        if self.can_coalesce(&other) {
            Ok(self.append(other))
        } else {
            Err((self, other))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(12, 123, 0, 123, None)] // second before first
    #[case(12, 123, 12, 123, None)] // second overlaps
    #[case(12, 123, 134, 123, None)] // second still overlaps
    #[case(12, 123, 135, 123, Some(246))] // second contiguous to first => OK
    #[case(12, 123, 136, 123, Some(247))] // small hole => OK
    #[case(12, 123, 135 + constants::MAX_HOLESIZE, 123, Some(246 + constants::MAX_HOLESIZE))] // maximum hole => OK
    #[case(12, 123, 136 + constants::MAX_HOLESIZE, 123, None)] // hole too large
    #[case(12, constants::LIMIT_PACK_READ - 15, constants::LIMIT_PACK_READ - 3, 15, Some(constants::LIMIT_PACK_READ))] // maximum length
    #[case(12, constants::LIMIT_PACK_READ - 15, constants::LIMIT_PACK_READ - 3, 16, None)] // exceeds limit to read
    #[case(12, constants::LIMIT_PACK_READ - 15, constants::LIMIT_PACK_READ, 12, Some(constants::LIMIT_PACK_READ))] // maximum length with hole
    #[case(12, constants::LIMIT_PACK_READ - 15, constants::LIMIT_PACK_READ + 1, 12, None)] // exceeds limit
    fn test_coalesce(
        #[case] offset1: u32,
        #[case] length1: u32,
        #[case] offset2: u32,
        #[case] length2: u32,
        #[case] expected: Option<u32>,
    ) {
        // helper to create BlobLocations
        let bl = |offset, length| {
            BlobLocations::from_blob_location(
                BlobLocation {
                    offset,
                    length,
                    uncompressed_length: None,
                },
                (),
            )
        };

        let coalesced_length = bl(offset1, length1)
            .coalesce(bl(offset2, length2))
            .ok()
            .map(|bl| bl.length);
        assert_eq!(coalesced_length, expected);
    }
}
