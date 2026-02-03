use bytes::Bytes;

use crate::{
    BlobId, RusticResult,
    backend::{cache::Cache, decrypt::DecryptBackend},
    crypto::aespoly1305::Key,
    index::GlobalIndex,
    repofile::{ConfigFile, KeyId},
};

/// A repository which is open, i.e. the password has been checked and the decryption key is available.
pub trait Open {
    /// Get the open status
    fn open_status(&self) -> &OpenStatus;
    /// Get the mutable open status
    fn open_status_mut(&mut self) -> &mut OpenStatus;
    /// Get the open status
    fn into_open_status(self) -> OpenStatus;
}

/// A repository which is indexed such that all tree blobs are contained in the index.
pub trait IndexedTree: Open {
    /// Returns the used indexes
    fn index(&self) -> &GlobalIndex;
}

/// A repository which is indexed such that all tree blobs are contained in the index
/// and additionally the `Id`s of data blobs are also contained in the index.
pub trait IndexedIds: IndexedTree {
    /// Turn the repository into the `IndexedTree` state by reading and storing a size-optimized index
    fn into_indexed_tree(self) -> IndexedTreesStatus;
}

/// A repository which is indexed such that all blob information is fully contained in the index.
pub trait IndexedFull: IndexedIds {
    /// Get a blob from the internal cache blob or insert it with the given function
    ///
    /// # Arguments
    ///
    /// * `id` - The [`Id`] of the blob to get
    /// * `with` - The function which fetches the blob from the repository if it is not contained in the cache
    ///
    /// # Errors
    ///
    /// * If the blob could not be fetched from the repository.
    ///
    /// # Returns
    ///
    /// The blob with the given id or the result of the given function if the blob is not contained in the cache
    /// and the function is called.
    fn get_blob_or_insert_with(
        &self,
        id: &BlobId,
        with: impl FnOnce() -> RusticResult<Bytes>,
    ) -> RusticResult<Bytes>;
}

/// Open Status: This repository is open, i.e. the password has been checked and the decryption key is available.
#[derive(Debug)]
pub struct OpenStatus {
    /// The cache
    pub(super) cache: Option<Cache>,
    /// The [`DecryptBackend`]
    pub(super) dbe: DecryptBackend<Key>,
    /// The [`ConfigFile`]
    pub(super) config: ConfigFile,
    /// The [`KeyId`] of the used key
    pub(super) key_id: Option<KeyId>,
}

impl Open for OpenStatus {
    fn open_status(&self) -> &OpenStatus {
        self
    }
    fn open_status_mut(&mut self) -> &mut OpenStatus {
        self
    }
    fn into_open_status(self) -> OpenStatus {
        self
    }
}

/// Indexed Tree Status: The repository is open and the index contains trees.
#[derive(Debug)]
pub struct IndexedTreesStatus {
    /// The open status
    pub(super) open: OpenStatus,
    /// The index backend
    pub(super) index: GlobalIndex,
}

impl Open for IndexedTreesStatus {
    fn open_status(&self) -> &OpenStatus {
        &self.open
    }
    fn open_status_mut(&mut self) -> &mut OpenStatus {
        &mut self.open
    }
    fn into_open_status(self) -> OpenStatus {
        self.open
    }
}

impl IndexedTree for IndexedTreesStatus {
    fn index(&self) -> &GlobalIndex {
        &self.index
    }
}

/// Indexed Tree Status: The repository is open and the index contains tree packs and the ids for data packs.
#[derive(Debug)]
pub struct IndexedIdsStatus {
    /// The open status
    pub(super) open: OpenStatus,
    /// The index backend
    pub(super) index: GlobalIndex,
}

impl Open for IndexedIdsStatus {
    fn open_status(&self) -> &OpenStatus {
        &self.open
    }
    fn open_status_mut(&mut self) -> &mut OpenStatus {
        &mut self.open
    }
    fn into_open_status(self) -> OpenStatus {
        self.open
    }
}

impl IndexedTree for IndexedIdsStatus {
    fn index(&self) -> &GlobalIndex {
        &self.index
    }
}

impl IndexedIds for IndexedIdsStatus {
    fn into_indexed_tree(self) -> IndexedTreesStatus {
        IndexedTreesStatus {
            open: self.open,
            index: self.index.drop_data(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// Defines a weighted cache with weight equal to the length of the blob size
pub(crate) struct BytesWeighter;

impl quick_cache::Weighter<BlobId, Bytes> for BytesWeighter {
    fn weight(&self, _key: &BlobId, val: &Bytes) -> u64 {
        u64::try_from(val.len())
            .expect("weight overflow in cache should not happen")
            // Be cautions out about zero weights!
            .max(1)
    }
}
/// Indexed Tree Status: The repository is open and the index contains trees.
///
#[derive(Debug)]
pub struct IndexedFullStatus {
    /// The open status
    pub(super) open: OpenStatus,
    /// The index backend
    pub(super) index: GlobalIndex,
    pub(super) cache: quick_cache::sync::Cache<BlobId, Bytes, BytesWeighter>,
}

impl Open for IndexedFullStatus {
    fn open_status(&self) -> &OpenStatus {
        &self.open
    }
    fn open_status_mut(&mut self) -> &mut OpenStatus {
        &mut self.open
    }
    fn into_open_status(self) -> OpenStatus {
        self.open
    }
}

impl IndexedTree for IndexedFullStatus {
    fn index(&self) -> &GlobalIndex {
        &self.index
    }
}

impl IndexedIds for IndexedFullStatus {
    fn into_indexed_tree(self) -> IndexedTreesStatus {
        IndexedTreesStatus {
            open: self.open,
            index: self.index.drop_data(),
        }
    }
}

impl IndexedFull for IndexedFullStatus {
    fn get_blob_or_insert_with(
        &self,
        id: &BlobId,
        with: impl FnOnce() -> RusticResult<Bytes>,
    ) -> RusticResult<Bytes> {
        self.cache.get_or_insert_with(id, with)
    }
}
