use std::{collections::HashSet, sync::Arc, thread::sleep, time::Duration};

use bytes::Bytes;

use crate::{
    backend::{FileType, decrypt::DecryptReadBackend},
    blob::{BlobId, BlobLocation, BlobType, DataId, tree::TreeId},
    error::{ErrorKind, RusticError, RusticResult},
    index::binarysorted::{Index, IndexCollector, IndexType},
    progress::Progress,
    repofile::{
        indexfile::{IndexBlob, IndexFile, IndexPack},
        packfile::PackId,
    },
};

pub(crate) mod binarysorted;
pub(crate) mod indexer;

/// An entry in the index
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    /// The type of the blob
    pub blob_type: BlobType,
    /// The pack the blob is in
    pub pack: PackId,
    /// The location of the blob in the pack
    pub location: BlobLocation,
}

impl IndexEntry {
    /// Create an [`IndexEntry`] from an [`IndexBlob`]
    ///
    /// # Arguments
    ///
    /// * `blob` - The [`IndexBlob`] to create the [`IndexEntry`] from
    /// * `pack` - The pack the blob is in
    #[must_use]
    pub const fn from_index_blob(blob: &IndexBlob, pack: PackId) -> Self {
        Self {
            blob_type: blob.tpe,
            pack,
            location: blob.location,
        }
    }

    /// Get a blob described by [`IndexEntry`] from the backend
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from
    ///
    /// # Errors
    ///
    // TODO:  add error! This function will return an error if the blob is not found in the backend.
    pub fn read_data<B: DecryptReadBackend>(&self, be: &B) -> RusticResult<Bytes> {
        let data = be.read_encrypted_partial(
            FileType::Pack,
            &self.pack,
            self.blob_type.is_cacheable(),
            self.location,
        )?;

        Ok(data)
    }

    /// Get the length of the data described by the [`IndexEntry`]
    #[must_use]
    pub const fn data_length(&self) -> u32 {
        self.location.data_length()
    }
}

/// The index of the repository
///
/// The index is a list of [`IndexEntry`]s
pub trait ReadIndex {
    /// Get an [`IndexEntry`] from the index
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Returns
    ///
    /// The [`IndexEntry`] - If it exists otherwise `None`
    fn get_id(&self, tpe: BlobType, id: &BlobId) -> Option<IndexEntry>;

    /// Get the total size of all blobs of the given type
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blobs
    fn total_size(&self, tpe: BlobType) -> u64;

    /// Check if the blob is in an active pack and can be reused by writers.
    /// Read lookups may also find blobs in packs retained for deletion.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    fn has(&self, tpe: BlobType, id: &BlobId) -> bool;

    /// Get a tree from the index
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the tree
    ///
    /// # Returns
    ///
    /// The [`IndexEntry`] of the tree if it exists otherwise `None`
    fn get_tree(&self, id: &TreeId) -> Option<IndexEntry> {
        self.get_id(BlobType::Tree, &BlobId::from(**id))
    }

    /// Get a data blob from the index
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the data blob
    ///
    /// # Returns
    ///
    /// The [`IndexEntry`] of the data blob if it exists otherwise `None`
    fn get_data(&self, id: &DataId) -> Option<IndexEntry> {
        self.get_id(BlobType::Data, &BlobId::from(**id))
    }

    /// Check if the tree is in an active pack and can be reused by writers.
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the tree
    ///
    /// # Returns
    ///
    /// `true` if the index contains the tree otherwise `false`
    fn has_tree(&self, id: &TreeId) -> bool {
        self.has(BlobType::Tree, &BlobId::from(**id))
    }

    /// Check if the data blob is in an active pack and can be reused by writers.
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the data blob
    ///
    /// # Returns
    ///
    /// `true` if the index contains the data blob otherwise `false`
    fn has_data(&self, id: &DataId) -> bool {
        self.has(BlobType::Data, &BlobId::from(**id))
    }

    /// Get a blob from the backend
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Errors
    ///
    /// * If the blob could not be found in the index
    fn blob_from_backend(
        &self,
        be: &impl DecryptReadBackend,
        tpe: BlobType,
        id: &BlobId,
    ) -> RusticResult<Bytes> {
        self.get_id(tpe, id).map_or_else(
            || {
                Err(RusticError::new(
                    ErrorKind::Internal,
                    "Blob `{id}` with type `{type}` not found in index",
                )
                .attach_context("id", id.to_string())
                .attach_context("type", tpe.to_string()))
            },
            |ie| ie.read_data(be),
        )
    }
}

/// A trait for a global index
pub trait ReadGlobalIndex: ReadIndex + Clone + Sync + Send + 'static {}

/// A global index
#[derive(Clone, Debug)]
pub struct GlobalIndex {
    /// The atomic reference counted, sharable index.
    index: Arc<Index>,
    fallback: Option<Arc<Index>>,
}

impl ReadIndex for GlobalIndex {
    /// Get an [`IndexEntry`] from the index
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Returns
    ///
    /// The [`IndexEntry`] - If it exists otherwise `None`
    fn get_id(&self, tpe: BlobType, id: &BlobId) -> Option<IndexEntry> {
        self.index.get_id(tpe, id).or_else(|| {
            self.fallback
                .as_ref()
                .and_then(|index| index.get_id(tpe, id))
        })
    }

    /// Get the total size of all blobs of the given type
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blobs
    fn total_size(&self, tpe: BlobType) -> u64 {
        self.index.total_size(tpe)
    }

    /// Check if the blob is in an active pack and can be reused by writers.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Returns
    ///
    /// `true` if the index contains the blob otherwise `false`
    fn has(&self, tpe: BlobType, id: &BlobId) -> bool {
        self.index.has(tpe, id)
    }
}

impl GlobalIndex {
    /// Create a new [`GlobalIndex`] from an [`Index`]
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend type
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from
    /// * `index` - The index to use
    pub fn new_from_index(index: Index) -> Self {
        Self {
            index: Arc::new(index),
            fallback: None,
        }
    }

    pub(crate) fn new_with_fallback(index: Index, fallback: Index) -> Self {
        Self {
            index: Arc::new(index),
            fallback: Some(Arc::new(fallback)),
        }
    }

    /// Create a new [`GlobalIndex`] from an [`IndexCollector`]
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from
    /// * `p` - The progress tracker
    /// * `collector` - The [`IndexCollector`] to use
    ///
    /// # Errors
    ///
    /// * If the index could not be read
    fn new_from_collector(
        be: &impl DecryptReadBackend,
        p: &Progress,
        mut collector: IndexCollector,
        mut fallback: IndexCollector,
    ) -> RusticResult<Self> {
        p.set_title("reading index...");
        for index in be.stream_all::<IndexFile>(p)? {
            let index = index?.1;
            collector.extend(index.packs);
            fallback.extend(index.packs_to_delete);
        }

        p.finish();

        Ok(Self::new_with_fallback(
            collector.into_index(),
            fallback.into_index(),
        ))
    }

    /// Create a new [`GlobalIndex`]
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from
    /// * `p` - The progress tracker
    pub fn new(be: &impl DecryptReadBackend, p: &Progress) -> RusticResult<Self> {
        Self::new_from_collector(
            be,
            p,
            IndexCollector::new(IndexType::Full),
            IndexCollector::new(IndexType::Full),
        )
    }

    /// Create a new [`GlobalIndex`] with only full trees
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to read from
    /// * `p` - The progress tracker
    ///
    /// # Errors
    ///
    /// * If the index could not be read
    pub fn only_full_trees(be: &impl DecryptReadBackend, p: &Progress) -> RusticResult<Self> {
        Self::new_from_collector(
            be,
            p,
            IndexCollector::new(IndexType::DataIds),
            IndexCollector::new(IndexType::OnlyTrees),
        )
    }

    pub(crate) fn into_packs(self) -> impl Iterator<Item = IndexPack> {
        let mut seen = HashSet::new();
        std::iter::once(self.index)
            .chain(self.fallback)
            .flat_map(|index| Self::unwrap_index(index).into_iter())
            .filter(move |pack| seen.insert(pack.id))
    }

    fn unwrap_index(index: Arc<Index>) -> Index {
        match Arc::try_unwrap(index) {
            Ok(index) => index,
            Err(arc) => {
                // Seems index is still in use; this could be due to some threads using it which didn't yet completely shut down.
                // sleep a bit to let threads using the index shut down, after this index should be available to unwrap
                sleep(Duration::from_millis(100));
                Arc::try_unwrap(arc).expect("index still in use")
            }
        }
    }

    pub(crate) fn drop_data(self) -> Self {
        Self {
            index: Arc::new(Self::unwrap_index(self.index).drop_data()),
            fallback: self
                .fallback
                .map(|index| Arc::new(Self::unwrap_index(index).drop_data())),
        }
    }
}

impl ReadGlobalIndex for GlobalIndex {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rstest::rstest;

    use crate::{
        ConfigOptions, Credentials, Id, KeyOptions, Repository, RepositoryBackends,
        RepositoryOptions,
        backend::decrypt::DecryptWriteBackend,
        blob::{BlobId, BlobType},
        repofile::{IndexFile, IndexPack, MasterKey},
        test_support::MemoryBackend,
    };

    use super::{
        GlobalIndex, IndexEntry, ReadIndex,
        binarysorted::{Index, IndexCollector, IndexType},
    };

    fn pack(blob_type: BlobType, pack_id: u8, blob_id: u8) -> IndexPack {
        let mut pack = IndexPack {
            id: Id::new([pack_id; 32]).into(),
            time: Some(jiff::Timestamp::from_second(1).unwrap()),
            ..Default::default()
        };
        pack.add(Id::new([blob_id; 32]).into(), blob_type, 0, 40, None);
        pack
    }

    fn index(index_type: IndexType, packs: Vec<IndexPack>) -> Index {
        let mut collector = IndexCollector::new(index_type);
        collector.extend(packs);
        collector.into_index()
    }

    #[rstest]
    #[case::tree(BlobType::Tree)]
    #[case::data(BlobType::Data)]
    fn reads_retained_blobs_without_reusing_them(#[case] blob_type: BlobType) {
        let active = pack(blob_type, 1, 11);
        let duplicate = pack(blob_type, 2, 11);
        let retired = pack(blob_type, 3, 12);
        let active_blob = active.blobs[0].id;
        let retired_blob = retired.blobs[0].id;
        let active_entry = IndexEntry::from_index_blob(&active.blobs[0], active.id);
        let retired_entry = IndexEntry::from_index_blob(&retired.blobs[0], retired.id);
        let active_size = u64::from(active.pack_size());
        let index = GlobalIndex::new_with_fallback(
            index(IndexType::Full, vec![active]),
            index(IndexType::Full, vec![duplicate, retired]),
        );

        assert_eq!(index.get_id(blob_type, &active_blob), Some(active_entry));
        assert!(index.has(blob_type, &active_blob));
        assert_eq!(index.get_id(blob_type, &retired_blob), Some(retired_entry));
        assert!(!index.has(blob_type, &retired_blob));
        assert_eq!(index.total_size(blob_type), active_size);
    }

    #[test]
    fn drop_data_preserves_active_and_retained_trees() {
        let active_tree = pack(BlobType::Tree, 1, 11);
        let active_data = pack(BlobType::Data, 2, 12);
        let retired_tree = pack(BlobType::Tree, 3, 13);
        let retired_data = pack(BlobType::Data, 4, 14);
        let active_tree_blob = active_tree.blobs[0].id;
        let active_data_blob = active_data.blobs[0].id;
        let retired_tree_blob = retired_tree.blobs[0].id;
        let retired_data_blob = retired_data.blobs[0].id;
        let active_tree_entry = IndexEntry::from_index_blob(&active_tree.blobs[0], active_tree.id);
        let retired_tree_entry =
            IndexEntry::from_index_blob(&retired_tree.blobs[0], retired_tree.id);
        let active_tree_size = u64::from(active_tree.pack_size());
        let index = GlobalIndex::new_with_fallback(
            index(IndexType::Full, vec![active_tree, active_data]),
            index(IndexType::Full, vec![retired_tree, retired_data]),
        )
        .drop_data();

        assert_eq!(
            index.get_id(BlobType::Tree, &active_tree_blob),
            Some(active_tree_entry),
        );
        assert_eq!(
            index.get_id(BlobType::Tree, &retired_tree_blob),
            Some(retired_tree_entry),
        );
        assert!(index.has(BlobType::Tree, &active_tree_blob));
        assert!(!index.has(BlobType::Tree, &retired_tree_blob));
        for blob in [active_data_blob, retired_data_blob] {
            assert!(index.get_id(BlobType::Data, &blob).is_none());
            assert!(!index.has(BlobType::Data, &blob));
        }
        assert_eq!(index.total_size(BlobType::Data), 0);
        assert_eq!(index.total_size(BlobType::Tree), active_tree_size);
    }

    #[test]
    fn repository_indexes_load_retained_blobs_without_changing_backup_data_mode()
    -> anyhow::Result<()> {
        let backends = RepositoryBackends::new(Arc::new(MemoryBackend::default()), None);
        let repo = Repository::new(&RepositoryOptions::default().no_cache(true), &backends)?.init(
            &Credentials::Masterkey(MasterKey::new()),
            &KeyOptions::default(),
            &ConfigOptions::default(),
        )?;
        let active_tree = pack(BlobType::Tree, 1, 11);
        let active_data = pack(BlobType::Data, 2, 12);
        let retired_tree = pack(BlobType::Tree, 3, 13);
        let retired_data = pack(BlobType::Data, 4, 14);
        let active_tree_blob = active_tree.blobs[0].id;
        let active_data_blob = active_data.blobs[0].id;
        let retired_tree_blob = retired_tree.blobs[0].id;
        let retired_data_blob = retired_data.blobs[0].id;
        _ = repo.dbe().save_file(&IndexFile {
            packs: vec![active_tree, active_data],
            packs_to_delete: vec![retired_tree, retired_data],
            ..Default::default()
        })?;

        let full = GlobalIndex::new(repo.dbe(), &repo.progress_hidden())?;
        let backup = GlobalIndex::only_full_trees(repo.dbe(), &repo.progress_hidden())?;
        for index in [&full, &backup] {
            assert!(index.get_id(BlobType::Tree, &active_tree_blob).is_some());
            assert!(index.get_id(BlobType::Tree, &retired_tree_blob).is_some());
            assert!(index.has(BlobType::Tree, &active_tree_blob));
            assert!(!index.has(BlobType::Tree, &retired_tree_blob));
            assert!(index.has(BlobType::Data, &active_data_blob));
            assert!(!index.has(BlobType::Data, &retired_data_blob));
        }
        for blob in [active_data_blob, retired_data_blob] {
            assert!(full.get_id(BlobType::Data, &blob).is_some());
            assert!(backup.get_id(BlobType::Data, &blob).is_none());
        }
        let missing: BlobId = Id::new([99; 32]).into();
        assert!(full.get_id(BlobType::Tree, &missing).is_none());
        assert!(!full.has(BlobType::Tree, &missing));
        Ok(())
    }
}
