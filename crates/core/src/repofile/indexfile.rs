use std::{cmp::Ordering, num::NonZeroU32};

use chrono::{DateTime, Local};
use serde_derive::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

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
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct IndexFile {
    /// which other index files are superseded by this (not actively used)
    pub supersedes: Option<Vec<IndexId>>,
    /// Index information about used packs
    pub packs: Vec<IndexPack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Index information about unused packs which are already marked for deletion
    pub packs_to_delete: Vec<IndexPack>,
}

impl RepoFile for IndexFile {
    /// The [`FileType`] associated with the [`IndexFile`]
    const TYPE: FileType = FileType::Index;
    type Id = IndexId;
}

impl IndexFile {
    /// Add a new pack to the index file
    ///
    /// # Arguments
    ///
    /// * `p` - The pack to add
    /// * `delete` - If the pack should be marked for deletion
    pub(crate) fn add(&mut self, p: IndexPack, delete: bool) {
        if delete {
            self.packs_to_delete.push(p);
        } else {
            self.packs.push(p);
        }
    }

    pub(crate) fn all_packs(self) -> impl Iterator<Item = (IndexPack, bool)> {
        self.packs
            .into_iter()
            .map(|pack| (pack, false))
            .chain(self.packs_to_delete.into_iter().map(|pack| (pack, true)))
    }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
/// Index information about a `pack`
pub struct IndexPack {
    /// pack Id
    pub id: PackId,
    /// Index information about contained blobs
    pub blobs: Vec<IndexBlob>,
    /// The pack creation time or time when the pack was marked for deletion
    pub time: Option<DateTime<Local>>,
    /// The pack size
    pub size: Option<u32>,
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
            offset,
            length,
            uncompressed_length,
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
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Copy)]
/// Index information about a `blob`
pub struct IndexBlob {
    /// Blob Id
    pub id: BlobId,
    #[serde(rename = "type")]
    /// Type of the blob
    pub tpe: BlobType,
    /// Offset of the blob within the `pack` file
    pub offset: u32,
    /// Length of the blob as stored within the `pack` file
    pub length: u32,
    /// Data length of the blob. This is only set if the blob is compressed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncompressed_length: Option<NonZeroU32>,
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
        self.offset.cmp(&other.offset)
    }
}
