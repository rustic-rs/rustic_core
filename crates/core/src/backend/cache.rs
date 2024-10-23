use std::{
    collections::HashMap,
    fs::{self, File},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::Arc,
};

use bytes::Bytes;
use dirs::cache_dir;
use log::{trace, warn};
use walkdir::WalkDir;

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    error::RusticResult,
    id::Id,
    repofile::configfile::RepositoryId,
};

/// [`CacheBackendErrorKind`] describes the errors that can be returned by a Caching action in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum CacheBackendErrorKind {
    /// Cache directory could not be determined, please set the environment variable XDG_CACHE_HOME or HOME!
    NoCacheDirectory,
    /// Error in cache backend {context} for {tpe:?} with {id}: {source}
    Io {
        context: String,
        source: std::io::Error,
        tpe: Option<FileType>,
        id: Id,
    },
    /// Ensuring tag failed for cache directory {path}: {source}
    EnsureTagFailed {
        source: std::io::Error,
        path: PathBuf,
    },
}

pub(crate) type CacheBackendResult<T> = Result<T, CacheBackendErrorKind>;

/// Backend that caches data.
///
/// This backend caches data in a directory.
/// It can be used to cache data from a remote backend.
///
/// # Type Parameters
///
/// * `BE` - The backend to cache.
#[derive(Clone, Debug)]
pub struct CachedBackend {
    /// The backend to cache.
    be: Arc<dyn WriteBackend>,
    /// The cache.
    cache: Cache,
}

impl CachedBackend {
    /// Create a new [`CachedBackend`] from a given backend.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend to cache.
    pub fn new_cache(be: Arc<dyn WriteBackend>, cache: Cache) -> Arc<dyn WriteBackend> {
        Arc::new(Self { be, cache })
    }
}

impl ReadBackend for CachedBackend {
    /// Returns the location of the backend as a String.
    fn location(&self) -> String {
        self.be.location()
    }

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// If the backend does not support listing files.
    ///
    /// # Returns
    ///
    /// A vector of tuples containing the id and size of the files.
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        let list = self.be.list_with_size(tpe)?;

        if tpe.is_cacheable() {
            if let Err(err) = self.cache.remove_not_in_list(tpe, &list) {
                warn!("Error in cache backend removing files {tpe:?}: {err}");
            }
        }

        Ok(list)
    }

    /// Reads full data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be read.
    ///
    /// # Returns
    ///
    /// The data read.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        if tpe.is_cacheable() {
            match self.cache.read_full(tpe, id) {
                Ok(Some(data)) => return Ok(data),
                Ok(None) => {}
                Err(err) => warn!("Error in cache backend reading {tpe:?},{id}: {err}"),
            }
            let res = self.be.read_full(tpe, id);
            if let Ok(data) = &res {
                if let Err(err) = self.cache.write_bytes(tpe, id, data) {
                    warn!("Error in cache backend writing {tpe:?},{id}: {err}");
                }
            }
            res
        } else {
            self.be.read_full(tpe, id)
        }
    }

    /// Reads partial data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be read.
    ///
    /// # Returns
    ///
    /// The data read.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        if cacheable || tpe.is_cacheable() {
            match self.cache.read_partial(tpe, id, offset, length) {
                Ok(Some(data)) => return Ok(data),
                Ok(None) => {}
                Err(err) => warn!("Error in cache backend reading {tpe:?},{id}: {err}"),
            };
            // read full file, save to cache and return partial content
            match self.be.read_full(tpe, id) {
                Ok(data) => {
                    let range = offset as usize..(offset + length) as usize;
                    if let Err(err) = self.cache.write_bytes(tpe, id, &data) {
                        warn!("Error in cache backend writing {tpe:?},{id}: {err}");
                    }
                    Ok(Bytes::copy_from_slice(&data.slice(range)))
                }
                error => error,
            }
        } else {
            self.be.read_partial(tpe, id, cacheable, offset, length)
        }
    }
    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        self.be.warm_up(tpe, id)
    }
}

impl WriteBackend for CachedBackend {
    /// Creates the backend.
    fn create(&self) -> RusticResult<()> {
        self.be.create()
    }

    /// Writes the given data to the given file.
    ///
    /// If the file is cacheable, it will also be written to the cache.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    /// * `buf` - The data to write.
    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        if cacheable || tpe.is_cacheable() {
            if let Err(err) = self.cache.write_bytes(tpe, id, &buf) {
                warn!("Error in cache backend writing {tpe:?},{id}: {err}");
            }
        }
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    /// Removes the given file.
    ///
    /// If the file is cacheable, it will also be removed from the cache.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        if cacheable || tpe.is_cacheable() {
            if let Err(err) = self.cache.remove(tpe, id) {
                warn!("Error in cache backend removing {tpe:?},{id}: {err}");
            }
        }
        self.be.remove(tpe, id, cacheable)
    }
}

/// Backend that caches data in a directory.
#[derive(Clone, Debug)]
pub struct Cache {
    /// The path to the cache.
    path: PathBuf,
}

impl Cache {
    /// Creates a new [`Cache`] with the given id.
    ///
    /// If no path is given, the cache will be created in the default cache directory.
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the cache.
    /// * `path` - The path to the cache.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::NoCacheDirectory`] - If no path is given and the default cache directory could not be determined.
    /// * [`CacheBackendErrorKind::FromIoError`] - If the cache directory could not be created.
    ///
    /// [`CacheBackendErrorKind::NoCacheDirectory`]: crate::error::CacheBackendErrorKind::NoCacheDirectory
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn new(id: RepositoryId, path: Option<PathBuf>) -> RusticResult<Self> {
        let mut path = if let Some(p) = path {
            p
        } else {
            let mut dir = cache_dir()
                .ok_or_else(|| CacheBackendErrorKind::NoCacheDirectory)
                .map_err(|_err| todo!("Error transition"))?;
            dir.push("rustic");
            dir
        };

        fs::create_dir_all(&path)
            .map_err(|err| CacheBackendErrorKind::Io {
                context: "while creating cache directory".into(),
                source: err,
                tpe: None,
                id: id.clone().into_inner(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        cachedir::ensure_tag(&path)
            .map_err(|err| CacheBackendErrorKind::EnsureTagFailed {
                source: err,
                path: path.clone(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        path.push(id.to_hex());

        fs::create_dir_all(&path)
            .map_err(|err| CacheBackendErrorKind::Io {
                context: "while creating cache directory with id".into(),
                source: err,
                tpe: None,
                id: id.clone().into_inner(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        Ok(Self { path })
    }

    /// Returns the path to the location of this [`Cache`].
    ///
    /// # Panics
    ///
    /// Panics if the path is not valid unicode.
    // TODO: Does this need to panic? Result?
    #[must_use]
    pub fn location(&self) -> &str {
        self.path.to_str().unwrap()
    }

    /// Returns the path to the directory of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the directory.
    /// * `id` - The id of the directory.
    #[must_use]
    pub fn dir(&self, tpe: FileType, id: &Id) -> PathBuf {
        let hex_id = id.to_hex();
        self.path.join(tpe.dirname()).join(&hex_id[0..2])
    }

    /// Returns the path to the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    #[must_use]
    pub fn path(&self, tpe: FileType, id: &Id) -> PathBuf {
        let hex_id = id.to_hex();
        self.path
            .join(tpe.dirname())
            .join(&hex_id[0..2])
            .join(hex_id)
    }

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the cache directory could not be read.
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    #[allow(clippy::unnecessary_wraps)]
    pub fn list_with_size(&self, tpe: FileType) -> RusticResult<HashMap<Id, u32>> {
        let path = self.path.join(tpe.dirname());

        let walker = WalkDir::new(path)
            .into_iter()
            .filter_map(walkdir::Result::ok)
            .filter(|e| {
                // only use files with length of 64 which are valid hex
                e.file_type().is_file()
                    && e.file_name().len() == 64
                    && e.file_name().is_ascii()
                    && e.file_name().to_str().is_some_and(|c| {
                        c.chars()
                            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                    })
            })
            .map(|e| {
                (
                    e.file_name().to_str().unwrap().parse().unwrap(),
                    // handle errors in metadata by returning a size of 0
                    e.metadata().map_or(0, |m| m.len().try_into().unwrap_or(0)),
                )
            });

        Ok(walker.collect())
    }

    /// Removes all files from the cache that are not in the given list.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files.
    /// * `list` - The list of files.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the cache directory could not be read.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn remove_not_in_list(&self, tpe: FileType, list: &Vec<(Id, u32)>) -> RusticResult<()> {
        let mut list_cache = self.list_with_size(tpe)?;
        // remove present files from the cache list
        for (id, size) in list {
            if let Some(cached_size) = list_cache.remove(id) {
                if &cached_size != size {
                    // remove cache files with non-matching size
                    self.remove(tpe, id)?;
                }
            }
        }
        // remove all remaining (i.e. not present in repo) cache files
        for id in list_cache.keys() {
            self.remove(tpe, id)?;
        }
        Ok(())
    }

    /// Reads full data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be read.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Option<Bytes>> {
        trace!("cache reading tpe: {:?}, id: {}", &tpe, &id);
        match fs::read(self.path(tpe, id)) {
            Ok(data) => {
                trace!("cache hit!");
                Ok(Some(data.into()))
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(CacheBackendErrorKind::Io {
                context: "while reading full data of file".into(),
                source: err,
                tpe: Some(tpe.clone()),
                id: id.clone(),
            })
            .map_err(|_err| todo!("Error transition")),
        }
    }

    /// Reads partial data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be read.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        offset: u32,
        length: u32,
    ) -> RusticResult<Option<Bytes>> {
        trace!(
            "cache reading tpe: {:?}, id: {}, offset: {}",
            &tpe,
            &id,
            &offset
        );
        let mut file = match File::open(self.path(tpe, id)) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(CacheBackendErrorKind::Io {
                    context: "while opening file".into(),
                    source: err,
                    tpe: Some(tpe.clone()),
                    id: id.clone(),
                })
                .map_err(|_err| todo!("Error transition"))
            }
        };
        _ = file
            .seek(SeekFrom::Start(u64::from(offset)))
            .map_err(|_err| todo!("Error transition"))?;
        let mut vec = vec![0; length as usize];
        file.read_exact(&mut vec)
            .map_err(|_err| todo!("Error transition"))?;
        trace!("cache hit!");
        Ok(Some(vec.into()))
    }

    /// Writes the given data to the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `buf` - The data to write.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be written.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn write_bytes(&self, tpe: FileType, id: &Id, buf: &Bytes) -> RusticResult<()> {
        trace!("cache writing tpe: {:?}, id: {}", &tpe, &id);

        fs::create_dir_all(self.dir(tpe, id))
            .map_err(|err| CacheBackendErrorKind::Io {
                context: "while creating directories".into(),
                source: err,
                tpe: Some(tpe.clone()),
                id: id.clone(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        let filename = self.path(tpe, id);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&filename)
            .map_err(|err| CacheBackendErrorKind::Io {
                context: "while opening file paths".into(),
                source: err,
                tpe: Some(tpe.clone()),
                id: id.clone(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        file.write_all(buf)
            .map_err(|err| CacheBackendErrorKind::Io {
                context: "while writing to buffer".into(),
                source: err,
                tpe: Some(tpe.clone()),
                id: id.clone(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        Ok(())
    }

    /// Removes the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`CacheBackendErrorKind::FromIoError`] - If the file could not be removed.
    ///
    /// [`CacheBackendErrorKind::FromIoError`]: crate::error::CacheBackendErrorKind::FromIoError
    pub fn remove(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        trace!("cache writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        fs::remove_file(&filename)
            .map_err(|err| CacheBackendErrorKind::Io {
                context: format!("while removing file: {filename:?}"),
                source: err,
                tpe: Some(tpe.clone()),
                id: id.clone(),
            })
            .map_err(|_err| todo!("Error transition"))?;

        Ok(())
    }
}
