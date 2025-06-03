use crate::{
    backend::decrypt::DecryptWriteBackend, blob::BlobId, error::RusticResult,
    repofile::indexfile::IndexPack,
};

use super::indexer::SharedIndexer;

/// The `Indexer` is responsible for indexing blobs.
#[derive(Debug, Clone)]
pub struct RepositoryIndexer<BE>
where
    BE: DecryptWriteBackend,
{
    /// The backend to write to.
    be: BE,
    /// The index file.
    raw_indexer: SharedIndexer,
}

impl<BE: DecryptWriteBackend> RepositoryIndexer<BE> {
    /// Creates a new `Indexer`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend type.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to write to.
    pub fn new(be: BE, raw_indexer: SharedIndexer) -> Self {
        Self { be, raw_indexer }
    }

    /// Adds a pack to the `Indexer`.
    ///
    /// # Arguments
    ///
    /// * `pack` - The pack to add.
    ///
    /// # Errors
    ///
    /// * If the index file could not be serialized.
    pub fn add(&self, pack: IndexPack) -> RusticResult<()> {
        self.add_with(pack, false)
    }

    /// Adds a pack to the `Indexer` and removes it from the backend.
    ///
    /// # Arguments
    ///
    /// * `pack` - The pack to add.
    ///
    /// # Errors
    ///
    /// * If the index file could not be serialized.
    pub fn add_remove(&self, pack: IndexPack) -> RusticResult<()> {
        self.add_with(pack, true)
    }

    /// Adds a pack to the `Indexer`.
    ///
    /// # Arguments
    ///
    /// * `pack` - The pack to add.
    /// * `delete` - Whether to delete the pack from the backend.
    ///
    /// # Errors
    ///
    /// * If the index file could not be serialized.
    pub fn add_with(&self, pack: IndexPack, delete: bool) -> RusticResult<()> {
        let res = {
            let mut raw_indexer = self.raw_indexer.write().unwrap();
            raw_indexer.add_with(pack, delete);
            raw_indexer.save_if_needed()
        };

        if let Some(file) = res {
            let _ = self.be.save_file(&file)?;
        }
        Ok(())
    }

    pub fn finalize(&self) -> RusticResult<()> {
        let res = self.raw_indexer.write().unwrap().finalize();

        if let Some(file) = res {
            let _ = self.be.save_file(&file)?;
        }
        Ok(())
    }

    /// Returns whether the given id is indexed. If not, mark it as indexed
    ///
    /// # Arguments
    ///
    /// * `id` - The id to check.
    pub fn has(&self, id: &BlobId) -> bool {
        self.raw_indexer.write().unwrap().has(id)
    }
}
