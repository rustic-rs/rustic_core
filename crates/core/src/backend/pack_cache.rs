//! Backend wrapper that caches pack files in memory for API-expensive backends like Google Drive.
//!
//! This backend is specifically designed to optimize performance for backends where
//! each read operation incurs significant overhead (e.g., one API call per read).
//! It caches entire pack files in memory on first partial read, then serves subsequent
//! reads from the cache.
//!
//! # Use Case
//!
//! During prune operations, rustic reads the same pack file many times in small chunks
//! (500-2000 bytes each). For local/S3 backends this is fine, but for Google Drive
//! each read is a separate API call, causing ~200+ API calls for just 13 pack files.
//!
//! This wrapper intercepts `read_partial` calls for `FileType::Pack`, caches the entire
//! pack file on first access, and serves subsequent reads from memory.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use log::{debug, trace};
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    error::RusticResult,
    id::Id,
};

/// Default number of pack files to cache in memory.
/// Each pack file is typically 4-50 MB, so 128 packs = ~2-6 GB memory usage.
const DEFAULT_CACHE_CAPACITY: usize = 128;

/// Backend wrapper that caches pack files in memory.
///
/// This is designed for backends like Google Drive where each read operation
/// is expensive (one API call). It caches entire pack files on first access
/// to avoid repeated API calls for the same file.
#[derive(Debug)]
pub struct PackCachingBackend {
    /// The inner backend to delegate to.
    be: Arc<dyn WriteBackend>,
    /// LRU cache for pack file contents, keyed by pack Id.
    cache: Mutex<LruCache<Id, Bytes>>,
}

impl PackCachingBackend {
    /// Create a new [`PackCachingBackend`] wrapping the given backend.
    ///
    /// Uses the default cache capacity of 16 pack files.
    #[must_use]
    #[allow(clippy::new_ret_no_self)]
    pub fn new(be: Arc<dyn WriteBackend>) -> Arc<dyn WriteBackend> {
        Self::with_capacity(be, DEFAULT_CACHE_CAPACITY)
    }

    /// Create a new [`PackCachingBackend`] with a custom cache capacity.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to wrap
    /// * `capacity` - Maximum number of pack files to cache
    ///
    /// # Panics
    ///
    /// Panics if `NonZeroUsize::new(1)` fails (should never happen), or if `NonZeroUsize::new(capacity)` fails and fallback also fails.
    #[must_use]
    pub fn with_capacity(be: Arc<dyn WriteBackend>, capacity: usize) -> Arc<dyn WriteBackend> {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1).unwrap());
        Arc::new(Self {
            be,
            cache: Mutex::new(LruCache::new(capacity)),
        })
    }

    /// Check if the backend location indicates it's a Google Drive backend.
    ///
    /// This is used to automatically wrap Google Drive backends with caching.
    #[must_use]
    pub fn is_gdrive_backend(be: &dyn WriteBackend) -> bool {
        let location = be.location();
        location.starts_with("opendal:gdrive:")
            || location.starts_with("opendal:google_drive:")
            || location.contains("drive.google.com")
    }

    /// Wrap the backend with pack caching if it's a Google Drive backend.
    ///
    /// This is a convenience method that automatically detects Google Drive
    /// backends and wraps them with caching.
    #[must_use]
    pub fn wrap_if_needed(be: Arc<dyn WriteBackend>) -> Arc<dyn WriteBackend> {
        if Self::is_gdrive_backend(be.as_ref()) {
            debug!(
                "Detected Google Drive backend, enabling pack file caching for {}",
                be.location()
            );
            Self::new(be)
        } else {
            be
        }
    }
}

impl ReadBackend for PackCachingBackend {
    fn location(&self) -> String {
        self.be.location()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        // For pack files, check cache first
        if tpe == FileType::Pack {
            let mut cache = self.cache.lock().unwrap();
            if let Some(data) = cache.get(id) {
                trace!("pack_cache hit for read_full: {id}");
                return Ok(data.clone());
            }
        }

        // Read from backend
        let data = self.be.read_full(tpe, id)?;

        // Cache pack files
        if tpe == FileType::Pack {
            let mut cache = self.cache.lock().unwrap();
            trace!("pack_cache storing full pack {id} ({} bytes)", data.len());
            let _ = cache.put(*id, data.clone());
        }

        Ok(data)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        // Only cache pack files - other file types are handled by the existing CachedBackend
        if tpe != FileType::Pack {
            return self.be.read_partial(tpe, id, cacheable, offset, length);
        }

        // Check cache first
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(data) = cache.get(id) {
                trace!("pack_cache hit for read_partial: {id} offset={offset} length={length}");
                let start = offset as usize;
                let end = (offset + length) as usize;
                // Handle case where requested range exceeds cached data
                let end = end.min(data.len());
                if start < data.len() {
                    return Ok(data.slice(start..end));
                }
            }
        }

        // Cache miss - read the FULL pack file and cache it
        debug!("pack_cache miss for {id}, fetching full pack file to cache");
        let full_data = self.be.read_full(tpe, id)?;

        // Store in cache
        {
            let mut cache = self.cache.lock().unwrap();
            trace!("pack_cache storing pack {id} ({} bytes)", full_data.len());
            let _ = cache.put(*id, full_data.clone());
        }

        // Return the requested slice
        let start = offset as usize;
        let end = (offset + length) as usize;
        let end = end.min(full_data.len());
        Ok(full_data.slice(start..end))
    }

    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        self.be.warm_up(tpe, id)
    }
}

impl WriteBackend for PackCachingBackend {
    fn create(&self) -> RusticResult<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        // Invalidate cache on write
        if tpe == FileType::Pack {
            let mut cache = self.cache.lock().unwrap();
            let _ = cache.pop(id);
        }
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        // Invalidate cache on remove
        if tpe == FileType::Pack {
            let mut cache = self.cache.lock().unwrap();
            let _ = cache.pop(id);
        }
        self.be.remove(tpe, id, cacheable)
    }
}

#[cfg(test)]
mod tests {
    // ...existing code...

    #[test]
    fn test_is_gdrive_backend() {
        // We can't easily create a mock backend, but we can at least test the function exists
        // and document the expected behavior
        // ...existing code...
    }
}
