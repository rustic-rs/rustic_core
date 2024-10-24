//! Module for backend related functionality.
pub(crate) mod cache;
pub(crate) mod childstdout;
pub(crate) mod decrypt;
pub(crate) mod dry_run;
pub(crate) mod hotcold;
pub(crate) mod ignore;
pub(crate) mod local_destination;
pub(crate) mod node;
pub(crate) mod stdin;
pub(crate) mod warm_up;

use std::{io::Read, num::TryFromIntError, ops::Deref, path::PathBuf, sync::Arc};

use bytes::Bytes;
use enum_map::Enum;
use log::trace;

#[cfg(test)]
use mockall::mock;

use serde_derive::{Deserialize, Serialize};

use crate::{
    backend::node::{Metadata, Node, NodeType},
    error::RusticResult,
    id::Id,
};

// #[derive(thiserror::Error, Debug, displaydoc::Display)]
// /// Experienced an error in the backend: `{0}`
// pub struct BackendDynError(pub Box<dyn std::error::Error + Send + Sync>);

/// [`BackendErrorKind`] describes the errors that can be returned by the various Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum BackendErrorKind {
    /// backend `{0:?}` is not supported!
    BackendNotSupported(String),
    /// no suitable id found for `{0}`
    NoSuitableIdFound(String),
    /// id `{0}` is not unique
    IdNotUnique(String),
    /// creating data in backend failed
    CreatingDataOnBackendFailed,
    /// writing bytes to backend failed
    WritingBytesToBackendFailed,
    /// removing data from backend failed
    RemovingDataFromBackendFailed,
    /// failed to list files on Backend
    ListingFilesOnBackendFailed,
    /// Path is not allowed: `{0:?}`
    PathNotAllowed(PathBuf),
    /// Backend location not convertible: `{location}`
    BackendLocationNotConvertible { location: String },
}

/// [`CryptBackendErrorKind`] describes the errors that can be returned by a Decryption action in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum CryptBackendErrorKind {
    /// decryption not supported for backend
    DecryptionNotSupportedForBackend,
    /// length of uncompressed data does not match!
    LengthOfUncompressedDataDoesNotMatch,
    /// failed to read encrypted data during full read
    DecryptionInFullReadFailed,
    /// failed to read encrypted data during partial read
    DecryptionInPartialReadFailed,
    /// decrypting from backend failed
    DecryptingFromBackendFailed,
    /// deserializing from bytes of JSON Text failed: `{0:?}`
    DeserializingFromBytesOfJsonTextFailed(serde_json::Error),
    /// failed to write data in crypt backend
    WritingDataInCryptBackendFailed,
    /// failed to list Ids
    ListingIdsInDecryptionBackendFailed,
    /// writing full hash failed in `CryptBackend`
    WritingFullHashFailed,
    /// decoding Zstd compressed data failed: `{0:?}`
    DecodingZstdCompressedDataFailed(std::io::Error),
    /// Serializing to JSON byte vector failed: `{0:?}`
    SerializingToJsonByteVectorFailed(serde_json::Error),
    /// encrypting data failed
    EncryptingDataFailed,
    /// Compressing and appending data failed: `{0:?}`
    CopyEncodingDataFailed(std::io::Error),
    /// conversion for integer failed: `{0:?}`
    IntConversionFailed(TryFromIntError),
    /// Extra verification failed: After decrypting and decompressing the data changed!
    ExtraVerificationFailed,
}

pub(crate) type BackendResult<T> = Result<T, BackendErrorKind>;
pub(crate) type CryptBackendResult<T> = Result<T, CryptBackendErrorKind>;

/// All [`FileType`]s which are located in separated directories
pub const ALL_FILE_TYPES: [FileType; 4] = [
    FileType::Key,
    FileType::Snapshot,
    FileType::Index,
    FileType::Pack,
];

/// Type for describing the kind of a file that can occur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Enum)]
pub enum FileType {
    /// Config file
    #[serde(rename = "config")]
    Config,
    /// Index
    #[serde(rename = "index")]
    Index,
    /// Keys
    #[serde(rename = "key")]
    Key,
    /// Snapshots
    #[serde(rename = "snapshot")]
    Snapshot,
    /// Data
    #[serde(rename = "pack")]
    Pack,
}

impl FileType {
    /// Returns the directory name of the file type.
    #[must_use]
    pub const fn dirname(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Snapshot => "snapshots",
            Self::Index => "index",
            Self::Key => "keys",
            Self::Pack => "data",
        }
    }

    /// Returns if the file type is cacheable.
    const fn is_cacheable(self) -> bool {
        match self {
            Self::Config | Self::Key | Self::Pack => false,
            Self::Snapshot | Self::Index => true,
        }
    }
}

/// Trait for backends that can read.
///
/// This trait is implemented by all backends that can read data.
pub trait ReadBackend: Send + Sync + 'static {
    /// Returns the location of the backend.
    fn location(&self) -> String;

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// If the files could not be listed.
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>>;

    /// Lists all files of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// If the files could not be listed.
    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
        Ok(self
            .list_with_size(tpe)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
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
    /// If the file could not be read.
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes>;

    /// Reads partial data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file should be cached.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    ///
    /// # Errors
    ///
    /// If the file could not be read.
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes>;

    /// Specify if the backend needs a warming-up of files before accessing them.
    fn needs_warm_up(&self) -> bool {
        false
    }

    /// Warm-up the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// If the file could not be read.
    fn warm_up(&self, _tpe: FileType, _id: &Id) -> RusticResult<()> {
        Ok(())
    }
}

/// Trait for Searching in a backend.
///
/// This trait is implemented by all backends that can be searched in.
///
/// # Note
///
/// This trait is used to find the id of a snapshot that contains a given file name.
pub trait FindInBackend: ReadBackend {
    /// Finds the id of the file starting with the given string.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of the strings.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `vec` - The strings to search for.
    ///
    /// # Errors
    ///
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// # Note
    ///
    /// This function is used to find the id of a snapshot.
    ///
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    fn find_starts_with<T: AsRef<str>>(&self, tpe: FileType, vec: &[T]) -> RusticResult<Vec<Id>> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum MapResult<T> {
            None,
            Some(T),
            NonUnique,
        }
        let mut results = vec![MapResult::None; vec.len()];
        for id in self.list(tpe)? {
            let id_hex = id.to_hex();
            for (i, v) in vec.iter().enumerate() {
                if id_hex.starts_with(v.as_ref()) {
                    if results[i] == MapResult::None {
                        results[i] = MapResult::Some(id);
                    } else {
                        results[i] = MapResult::NonUnique;
                    }
                }
            }
        }

        results
            .into_iter()
            .enumerate()
            .map(|(i, id)| match id {
                MapResult::Some(id) => Ok(id),
                MapResult::None => Err(BackendErrorKind::NoSuitableIdFound(
                    (vec[i]).as_ref().to_string(),
                ))
                .map_err(|_err| todo!("Error transition")),
                MapResult::NonUnique => {
                    Err(BackendErrorKind::IdNotUnique((vec[i]).as_ref().to_string()))
                        .map_err(|_err| todo!("Error transition"))
                }
            })
            .collect()
    }

    /// Finds the id of the file starting with the given string.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The string to search for.
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    fn find_id(&self, tpe: FileType, id: &str) -> RusticResult<Id> {
        Ok(self.find_ids(tpe, &[id.to_string()])?.remove(0))
    }

    /// Finds the ids of the files starting with the given strings.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of the strings.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `ids` - The strings to search for.
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    fn find_ids<T: AsRef<str>>(&self, tpe: FileType, ids: &[T]) -> RusticResult<Vec<Id>> {
        ids.iter()
            .map(|id| id.as_ref().parse())
            .collect::<RusticResult<Vec<_>>>()
            .or_else(|err|{
                trace!("no valid IDs given: {err}, searching for ID starting with given strings instead");
                self.find_starts_with(tpe, ids)})
    }
}

impl<T: ReadBackend> FindInBackend for T {}

/// Trait for backends that can write.
/// This trait is implemented by all backends that can write data.
pub trait WriteBackend: ReadBackend {
    /// Creates a new backend.
    ///
    /// # Errors
    ///
    /// If the backend could not be created.
    ///
    /// # Returns
    ///
    /// The result of the creation.
    fn create(&self) -> RusticResult<()> {
        Ok(())
    }

    /// Writes bytes to the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the data can be cached.
    /// * `buf` - The data to write.
    ///
    /// # Errors
    ///
    /// If the data could not be written.
    ///
    /// # Returns
    ///
    /// The result of the write.
    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()>;

    /// Removes the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    ///
    /// # Errors
    ///
    /// If the file could not be removed.
    ///
    /// # Returns
    ///
    /// The result of the removal.
    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()>;
}

#[cfg(test)]
mock! {
    Backend {}

    impl ReadBackend for Backend{
        fn location(&self) -> String;
        fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>>;
        fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes>;
        fn read_partial(
            &self,
            tpe: FileType,
            id: &Id,
            cacheable: bool,
            offset: u32,
            length: u32,
        ) -> RusticResult<Bytes>;
    }

    impl WriteBackend for Backend {
        fn create(&self) -> RusticResult<()>;
        fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()>;
        fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()>;
    }
}

impl WriteBackend for Arc<dyn WriteBackend> {
    fn create(&self) -> RusticResult<()> {
        self.deref().create()
    }
    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        self.deref().write_bytes(tpe, id, cacheable, buf)
    }
    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        self.deref().remove(tpe, id, cacheable)
    }
}

impl ReadBackend for Arc<dyn WriteBackend> {
    fn location(&self) -> String {
        self.deref().location()
    }
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.deref().list_with_size(tpe)
    }
    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
        self.deref().list(tpe)
    }
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.deref().read_full(tpe, id)
    }
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        self.deref()
            .read_partial(tpe, id, cacheable, offset, length)
    }
}

impl std::fmt::Debug for dyn WriteBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WriteBackend{{{}}}", self.location())
    }
}

/// Information about an entry to be able to open it.
///
/// # Type Parameters
///
/// * `O` - The type of the open information.
#[derive(Debug, Clone)]
pub struct ReadSourceEntry<O> {
    /// The path of the entry.
    pub path: PathBuf,

    /// The node information of the entry.
    pub node: Node,

    /// Information about how to open the entry.
    pub open: Option<O>,
}

impl<O> ReadSourceEntry<O> {
    fn from_path(path: PathBuf, open: Option<O>) -> RusticResult<Self> {
        let node = Node::new_node(
            path.file_name()
                .ok_or_else(|| BackendErrorKind::PathNotAllowed(path.clone()))
                .map_err(|_err| todo!("Error transition"))?,
            NodeType::File,
            Metadata::default(),
        );
        Ok(Self { path, node, open })
    }
}

/// Trait for backends that can read and open sources.
/// This trait is implemented by all backends that can read data and open from a source.
pub trait ReadSourceOpen {
    /// The Reader used for this source
    type Reader: Read + Send + 'static;

    /// Opens the source.
    ///
    /// # Errors
    ///
    /// If the source could not be opened.
    ///
    /// # Result
    ///
    /// The reader used to read from the source.
    fn open(self) -> RusticResult<Self::Reader>;
}

/// blanket implementation for readers
impl<T: Read + Send + 'static> ReadSourceOpen for T {
    type Reader = T;
    fn open(self) -> RusticResult<Self::Reader> {
        Ok(self)
    }
}

/// Trait for backends that can read from a source.
///
/// This trait is implemented by all backends that can read data from a source.
pub trait ReadSource: Sync + Send {
    /// The type used to handle open source files
    type Open: ReadSourceOpen;
    /// The iterator we use to iterate over the source entries
    type Iter: Iterator<Item = RusticResult<ReadSourceEntry<Self::Open>>>;

    /// Returns the size of the source.
    ///
    /// # Errors
    ///
    /// If the size could not be determined.
    ///
    /// # Returns
    ///
    /// The size of the source, if it is known.
    fn size(&self) -> RusticResult<Option<u64>>;

    /// Returns an iterator over the entries of the source.
    fn entries(&self) -> Self::Iter;
}

/// Trait for backends that can write to a source.
///
/// This trait is implemented by all backends that can write data to a source.
pub trait WriteSource: Clone {
    /// Create a new source.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the source.
    /// * `node` - The node information of the source.
    fn create<P: Into<PathBuf>>(&self, path: P, node: Node);

    /// Set the metadata of a source.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the source.
    /// * `node` - The node information of the source.
    fn set_metadata<P: Into<PathBuf>>(&self, path: P, node: Node);

    /// Write data to a source at the given offset.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the source.
    /// * `offset` - The offset to write at.
    /// * `data` - The data to write.
    fn write_at<P: Into<PathBuf>>(&self, path: P, offset: u64, data: Bytes);
}

/// The backends a repository can be initialized and operated on
///
/// # Note
///
/// This struct is used to initialize a [`Repository`].
///
/// [`Repository`]: crate::Repository
#[derive(Debug, Clone)]
pub struct RepositoryBackends {
    /// The main repository of this [`RepositoryBackends`].
    repository: Arc<dyn WriteBackend>,

    /// The hot repository of this [`RepositoryBackends`].
    repo_hot: Option<Arc<dyn WriteBackend>>,
}

impl RepositoryBackends {
    /// Creates a new [`RepositoryBackends`].
    ///
    /// # Arguments
    ///
    /// * `repository` - The main repository of this [`RepositoryBackends`].
    /// * `repo_hot` - The hot repository of this [`RepositoryBackends`].
    pub fn new(repository: Arc<dyn WriteBackend>, repo_hot: Option<Arc<dyn WriteBackend>>) -> Self {
        Self {
            repository,
            repo_hot,
        }
    }

    /// Returns the repository of this [`RepositoryBackends`].
    #[must_use]
    pub fn repository(&self) -> Arc<dyn WriteBackend> {
        self.repository.clone()
    }

    /// Returns the hot repository of this [`RepositoryBackends`].
    #[must_use]
    pub fn repo_hot(&self) -> Option<Arc<dyn WriteBackend>> {
        self.repo_hot.clone()
    }
}
