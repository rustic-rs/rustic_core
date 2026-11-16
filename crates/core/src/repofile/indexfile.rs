use crate::{blob::BlobLocation, repofile::RusticTime};
use std::{cmp::Ordering, num::NonZeroU32};

use displaydoc::Display;
use jiff::Timestamp;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none};

use crate::{
    backend::FileType,
    blob::{BlobId, BlobType},
    impl_repoid,
    repofile::{RepoFile, packfile::PackHeaderRef},
};

use super::packfile::PackId;

impl_repoid!(IndexId, FileType::Index);

/// Index files describe index information about multiple `pack` files.
///
/// They are usually stored in the repository under `/index/<ID>`
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct IndexFile {
    /// which other index files are superseded by this (not actively used)
    pub supersedes: Option<Vec<IndexId>>,
    /// Index information about used packs
    pub packs: Vec<IndexPack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Index information about unused packs which are already marked for deletion
    pub packs_to_delete: Vec<IndexPack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Index information about packs which are marked for recovery
    pub packs_to_recover: Vec<IndexPack>,
}

impl RepoFile for IndexFile {
    /// The [`FileType`] associated with the [`IndexFile`]
    const TYPE: FileType = FileType::Index;
    type Id = IndexId;
}

impl IndexFile {
    #[must_use]
    /// the total count of all packs (marked or unmarked)
    pub fn len(&self) -> usize {
        self.packs.len() + self.packs_to_delete.len() + self.packs_to_recover.len()
    }

    #[must_use]
    /// returns whether the index contains no packs (marked or unmarked) at all
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add a new pack to the index file
    ///
    /// # Arguments
    ///
    /// * `p` - The pack to add
    /// * `delete` - If the pack should be marked for deletion
    pub(crate) fn add(&mut self, p: IndexPack) {
        match p.mark {
            IndexMarkType::None => self.packs.push(p),
            IndexMarkType::Delete => self.packs_to_delete.push(p),
            IndexMarkType::Recover => self.packs_to_recover.push(p),
        }
    }

    pub(crate) fn all_packs(self) -> impl Iterator<Item = IndexPack> {
        self.packs
            .into_iter()
            .map(|mut pack| {
                pack.mark = IndexMarkType::None;
                pack
            })
            .chain(self.packs_to_delete.into_iter().map(|mut pack| {
                pack.mark = IndexMarkType::Delete;
                pack
            }))
            .chain(self.packs_to_recover.into_iter().map(|mut pack| {
                pack.mark = IndexMarkType::Recover;
                pack
            }))
    }
}

#[derive(Clone, Copy, Debug, Display, Default, PartialEq, Eq, PartialOrd, Ord)]
/// The possible marks set on a pack file in the index
pub enum IndexMarkType {
    /// no mark
    #[default]
    None,
    /// marked to-recover
    Recover,
    /// marked to-delete
    Delete,
}

impl IndexMarkType {
    pub(crate) fn has_mark(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
/// Index information about a `pack`
pub struct IndexPack {
    /// pack Id
    pub id: PackId,
    /// Index information about contained blobs
    pub blobs: Vec<IndexBlob>,
    #[serde_as(as = "Option<RusticTime>")]
    /// The pack creation time or time when the pack was marked for deletion
    pub time: Option<Timestamp>,
    /// The pack size
    pub size: Option<u32>,
    /// the mark of the pack file - is not serialized as repofile, instead separate lists packs-to-delete and packs-to-recover are used
    #[serde(skip)]
    pub(crate) mark: IndexMarkType,
}

impl IndexPack {
    /// Add a new blob to the pack
    ///
    /// # Arguments
    ///
    /// * `id` - The blob id
    /// * `tpe` - The blob type
    /// * `offset` - The blob offset within the pack
    /// * `length` - The blob length within the pack
    /// * `uncompressed_length` - The blob uncompressed length within the pack
    pub(crate) fn add(
        &mut self,
        id: BlobId,
        tpe: BlobType,
        offset: u32,
        length: u32,
        uncompressed_length: Option<NonZeroU32>,
    ) {
        self.blobs.push(IndexBlob {
            id,
            tpe,
            location: BlobLocation {
                offset,
                length,
                uncompressed_length,
            },
        });
    }

    /// Calculate the pack size from the contained blobs
    #[must_use]
    pub fn pack_size(&self) -> u32 {
        self.size
            .unwrap_or_else(|| PackHeaderRef::from_index_pack(self).pack_size())
    }

    /// Returns the blob type of the pack.
    ///
    /// # Note
    ///
    /// Only packs with identical blob types are allowed.
    #[must_use]
    pub fn blob_type(&self) -> BlobType {
        // TODO: This is a hack to support packs without blobs (e.g. when deleting unreferenced files)
        if self.blobs.is_empty() {
            BlobType::Data
        } else {
            self.blobs[0].tpe
        }
    }

    pub(crate) fn has_mark(&self) -> bool {
        self.mark.has_mark()
    }

    /// sets the time, but only if it is not already set
    pub fn set_time_if_unset(&mut self, time: Timestamp) {
        self.time = self.time.or(Some(time));
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Copy)]
/// Index information about a `blob`
pub struct IndexBlob {
    /// Blob Id
    pub id: BlobId,
    #[serde(rename = "type")]
    /// Type of the blob
    pub tpe: BlobType,

    #[serde(flatten)]
    /// Location of the blob within the `pack` file
    pub location: BlobLocation,
}

impl PartialOrd<Self> for IndexBlob {
    /// Compare two blobs by their offset
    ///
    /// # Arguments
    ///
    /// * `other` - The other blob to compare to
    ///
    /// # Returns
    ///
    /// The ordering of the two blobs
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IndexBlob {
    /// Compare two blobs by their offset
    ///
    /// # Arguments
    ///
    /// * `other` - The other blob to compare to
    ///
    /// # Returns
    ///
    /// The ordering of the two blobs
    fn cmp(&self, other: &Self) -> Ordering {
        self.location.cmp(&other.location)
    }
}
