//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use std::{
    error::Error,
    ffi::OsString,
    num::{ParseFloatError, ParseIntError, TryFromIntError},
    ops::RangeInclusive,
    path::{PathBuf, StripPrefixError},
    process::ExitStatus,
    str::Utf8Error,
};

#[cfg(not(windows))]
use nix::errno::Errno;

use aes256ctr_poly1305aes::aead;
use chrono::OutOfRangeError;
use displaydoc::Display;
use thiserror::Error;

use crate::{
    backend::node::NodeType,
    blob::{tree::TreeId, BlobId},
    repofile::{indexfile::IndexPack, packfile::PackId},
};

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T> = Result<T, RusticError>;

// [`Error`] is public, but opaque and easy to keep compatible.
#[derive(Error, Debug)]
#[error(transparent)]
/// Errors that can result from rustic.
pub struct RusticError(#[from] pub(crate) RusticErrorKind);

// Accessors for anything we do want to expose publicly.
impl RusticError {
    /// Expose the inner error kind.
    ///
    /// This is useful for matching on the error kind.
    pub fn into_inner(self) -> RusticErrorKind {
        self.0
    }

    /// Checks if the error is due to an incorrect password
    pub fn is_incorrect_password(&self) -> bool {
        matches!(
            self.0,
            RusticErrorKind::Repository(RepositoryErrorKind::IncorrectPassword)
        )
    }

    /// Get the corresponding backend error, if error is caused by the backend.
    ///
    /// Returns `anyhow::Error`; you need to cast this to the real backend error type
    pub fn backend_error(&self) -> Option<&anyhow::Error> {
        if let RusticErrorKind::Backend(error) = &self.0 {
            Some(error)
        } else {
            None
        }
    }
}

/// [`RusticErrorKind`] describes the errors that can happen while executing a high-level command.
///
/// This is a non-exhaustive enum, so additional variants may be added in future. It is
/// recommended to match against the wildcard `_` instead of listing all possible variants,
/// to avoid problems when new variants are added.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum RusticErrorKind {
    /// [`CommandErrorKind`] describes the errors that can happen while executing a high-level command
    #[error(transparent)]
    Command(#[from] CommandErrorKind),

    /// [`CryptoErrorKind`] describes the errors that can happen while dealing with Cryptographic functions
    #[error(transparent)]
    Crypto(#[from] CryptoErrorKind),

    /// [`PolynomialErrorKind`] describes the errors that can happen while dealing with Polynomials
    #[error(transparent)]
    Polynomial(#[from] PolynomialErrorKind),

    /// [`IdErrorKind`] describes the errors that can be returned by processing IDs
    #[error(transparent)]
    Id(#[from] IdErrorKind),

    /// [`RepositoryErrorKind`] describes the errors that can be returned by processing Repositories
    #[error(transparent)]
    Repository(#[from] RepositoryErrorKind),

    /// [`IndexErrorKind`] describes the errors that can be returned by processing Indizes
    #[error(transparent)]
    Index(#[from] IndexErrorKind),

    /// describes the errors that can be returned by the various Backends
    #[error(transparent)]
    Backend(#[from] anyhow::Error),

    /// [`BackendAccessErrorKind`] describes the errors that can be returned by accessing the various Backends
    #[error(transparent)]
    BackendAccess(#[from] BackendAccessErrorKind),

    /// [`ConfigFileErrorKind`] describes the errors that can be returned for `ConfigFile`s
    #[error(transparent)]
    ConfigFile(#[from] ConfigFileErrorKind),

    /// [`KeyFileErrorKind`] describes the errors that can be returned for `KeyFile`s
    #[error(transparent)]
    KeyFile(#[from] KeyFileErrorKind),

    /// [`PackFileErrorKind`] describes the errors that can be returned for `PackFile`s
    #[error(transparent)]
    PackFile(#[from] PackFileErrorKind),

    /// [`SnapshotFileErrorKind`] describes the errors that can be returned for `SnapshotFile`s
    #[error(transparent)]
    SnapshotFile(#[from] SnapshotFileErrorKind),

    /// [`PackerErrorKind`] describes the errors that can be returned for a Packer
    #[error(transparent)]
    Packer(#[from] PackerErrorKind),

    /// [`FileErrorKind`] describes the errors that can happen while dealing with files during restore/backups
    #[error(transparent)]
    File(#[from] FileErrorKind),

    /// [`TreeErrorKind`] describes the errors that can come up dealing with Trees
    #[error(transparent)]
    Tree(#[from] TreeErrorKind),

    /// [`CacheBackendErrorKind`] describes the errors that can be returned by a Caching action in Backends
    #[error(transparent)]
    CacheBackend(#[from] CacheBackendErrorKind),

    /// [`CryptBackendErrorKind`] describes the errors that can be returned by a Decryption action in Backends
    #[error(transparent)]
    CryptBackend(#[from] CryptBackendErrorKind),

    /// [`IgnoreErrorKind`] describes the errors that can be returned by a Ignore action in Backends
    #[error(transparent)]
    Ignore(#[from] IgnoreErrorKind),

    /// [`LocalDestinationErrorKind`] describes the errors that can be returned by an action on the local filesystem as Destination
    #[error(transparent)]
    LocalDestination(#[from] LocalDestinationErrorKind),

    /// [`NodeErrorKind`] describes the errors that can be returned by an action utilizing a node in Backends
    #[error(transparent)]
    Node(#[from] NodeErrorKind),

    /// [`StdInErrorKind`] describes the errors that can be returned while dealing IO from CLI
    #[error(transparent)]
    StdIn(#[from] StdInErrorKind),

    /// [`ArchiverErrorKind`] describes the errors that can be returned from the archiver
    #[error(transparent)]
    ArchiverError(#[from] ArchiverErrorKind),

    /// [`VfsErrorKind`] describes the errors that can be returned from the Virtual File System
    #[error(transparent)]
    VfsError(#[from] VfsErrorKind),

    /// [`std::io::Error`]
    #[error(transparent)]
    StdIo(#[from] std::io::Error),
}

/// [`CommandErrorKind`] describes the errors that can happen while executing a high-level command
#[derive(Error, Debug, Display)]
pub enum CommandErrorKind {
    /// path is no dir: `{0:?}`
    PathIsNoDir(String),
    /// used blobs are missing: blob {0} doesn't existing
    BlobsMissing(BlobId),
    /// used pack {0}: size does not match! Expected size: {1}, real size: {2}
    PackSizeNotMatching(PackId, u32, u32),
    /// "used pack {0} does not exist!
    PackNotExisting(PackId),
    /// pack {0} got no decision what to do
    NoDecision(PackId),
    /// {0:?}
    FromParseFloatError(#[from] ParseFloatError),
    /// {0:?}
    FromParseIntError(#[from] ParseIntError),
    /// {0}
    FromByteSizeParser(String),
    /// --repack-uncompressed makes no sense for v1 repo!
    RepackUncompressedRepoV1,
    /// datetime out of range: `{0:?}`
    FromOutOfRangeError(#[from] OutOfRangeError),
    /// node type {0:?} not supported by dump
    DumpNotSupported(NodeType),
    /// {0:?}
    FromJsonError(#[from] serde_json::Error),
    /// version {0} is not supported. Allowed values: {1:?}
    VersionNotSupported(u32, RangeInclusive<u32>),
    /// cannot downgrade version from {0} to {1}
    CannotDowngrade(u32, u32),
    /// compression level {0} is not supported for repo v1
    NoCompressionV1Repo(i32),
    /// compression level {0} is not supported. Allowed values: {1:?}
    CompressionLevelNotSupported(i32, RangeInclusive<i32>),
    /// Size is too large: {0}
    SizeTooLarge(bytesize::ByteSize),
    /// min_packsize_tolerate_percent must be <= 100
    MinPackSizeTolerateWrong,
    /// max_packsize_tolerate_percent must be >= 100 or 0"
    MaxPackSizeTolerateWrong,
    /// error creating {0:?}: {1:?}
    ErrorCreating(PathBuf, Box<RusticError>),
    /// error collecting information for {0:?}: {1:?}
    ErrorCollecting(PathBuf, Box<RusticError>),
    /// error setting length for {0:?}: {1:?}
    ErrorSettingLength(PathBuf, Box<RusticError>),
    /// {0:?}
    FromRayonError(#[from] rayon::ThreadPoolBuildError),
    /// conversion from integer failed: `{0:?}`
    ConversionFromIntFailed(TryFromIntError),
    /// {0} is not allowed on an append-only repository
    NotAllowedWithAppendOnly(String),
    /// Specify one of the keep-* options for forget! Please use keep-none to keep no snapshot.
    NoKeepOption,
    /// {0:?}
    FromParseError(#[from] shell_words::ParseError),
    /// Pack Id {0:?} not found in index
    PackIdNotFoundinIndex(PackId),
    /// Blob Id {0:?} not found in index
    BlobIdNotFoundinIndex(BlobId),
    /// No locking capability configured for the backend
    NoLockingConfigured,
}

/// [`CryptoErrorKind`] describes the errors that can happen while dealing with Cryptographic functions
#[derive(Error, Debug, Display, Copy, Clone)]
pub enum CryptoErrorKind {
    /// data decryption failed
    DataDecryptionFailed(aead::Error),
    /// data encryption failed
    DataEncryptionFailed,
    /// crypto key too short
    CryptoKeyTooShort,
}

/// [`PolynomialErrorKind`] describes the errors that can happen while dealing with Polynomials
#[derive(Error, Debug, Display, Copy, Clone)]
pub enum PolynomialErrorKind {
    /// no suitable polynomial found
    NoSuitablePolynomialFound,
}

/// [`FileErrorKind`] describes the errors that can happen while dealing with files during restore/backups
#[derive(Error, Debug, Display)]
pub enum FileErrorKind {
    /// transposing an Option of a Result into a Result of an Option failed: `{0:?}`
    TransposingOptionResultFailed(std::io::Error),
    /// conversion from `u64` to `usize` failed: `{0:?}`
    ConversionFromU64ToUsizeFailed(TryFromIntError),
}

/// [`IdErrorKind`] describes the errors that can be returned by processing IDs
#[derive(Error, Debug, Display, Copy, Clone)]
pub enum IdErrorKind {
    /// Hex decoding error: `{0:?}`
    HexError(hex::FromHexError),
}

/// [`RepositoryErrorKind`] describes the errors that can be returned by processing Repositories
#[derive(Error, Debug, Display)]
pub enum RepositoryErrorKind {
    /// No repository given. Please use the --repository option.
    NoRepositoryGiven,
    /// No password given. Please use one of the --password-* options.
    NoPasswordGiven,
    /// warm-up command must contain %id!
    NoIDSpecified,
    /// error opening password file `{0:?}`
    OpeningPasswordFileFailed(std::io::Error),
    /// No repository config file found. Is there a repo at {0}?
    NoRepositoryConfigFound(String),
    /// More than one repository config file at {0}. Aborting.
    MoreThanOneRepositoryConfig(String),
    /// keys from repo and repo-hot do not match for {0}. Aborting.
    KeysDontMatchForRepositories(String),
    /// repository is a hot repository!\nPlease use as --repo-hot in combination with the normal repo. Aborting.
    HotRepositoryFlagMissing,
    /// repo-hot is not a hot repository! Aborting.
    IsNotHotRepository,
    /// incorrect password!
    IncorrectPassword,
    /// error running the password command
    PasswordCommandExecutionFailed,
    /// error reading password from command
    ReadingPasswordFromCommandFailed,
    /// running command {0}:{1} was not successful: {2}
    CommandExecutionFailed(String, String, std::io::Error),
    /// running command {0}:{1} returned status: {2}
    CommandErrorStatus(String, String, ExitStatus),
    /// error listing the repo config file
    ListingRepositoryConfigFileFailed,
    /// error listing the repo keys
    ListingRepositoryKeysFailed,
    /// error listing the hot repo keys
    ListingHotRepositoryKeysFailed,
    /// error accessing config file
    AccessToConfigFileFailed,
    /// {0:?}
    FromThreadPoolbilderError(rayon::ThreadPoolBuildError),
    /// reading Password failed: `{0:?}`
    ReadingPasswordFromReaderFailed(std::io::Error),
    /// reading Password from prompt failed: `{0:?}`
    ReadingPasswordFromPromptFailed(std::io::Error),
    /// Config file already exists. Aborting.
    ConfigFileExists,
    /// did not find id {0} in index
    IdNotFound(BlobId),
    /// no suitable backend type found
    NoBackendTypeGiven,
}

/// [`IndexErrorKind`] describes the errors that can be returned by processing Indizes
#[derive(Error, Debug, Display, Clone, Copy)]
pub enum IndexErrorKind {
    /// blob not found in index
    BlobInIndexNotFound,
    /// failed to get a blob from the backend
    GettingBlobIndexEntryFromBackendFailed,
    /// saving IndexFile failed
    SavingIndexFileFailed,
}

/// [`BackendAccessErrorKind`] describes the errors that can be returned by the various Backends
#[derive(Error, Debug, Display)]
pub enum BackendAccessErrorKind {
    /// backend {0:?} is not supported!
    BackendNotSupported(String),
    /// backend {0} cannot be loaded: {1:?}
    BackendLoadError(String, anyhow::Error),
    /// no suitable id found for {0}
    NoSuitableIdFound(String),
    /// id {0} is not unique
    IdNotUnique(String),
    /// {0:?}
    #[error(transparent)]
    FromIoError(#[from] std::io::Error),
    /// {0:?}
    #[error(transparent)]
    FromTryIntError(#[from] TryFromIntError),
    /// {0:?}
    #[error(transparent)]
    FromLocalError(#[from] LocalDestinationErrorKind),
    /// {0:?}
    #[error(transparent)]
    FromIdError(#[from] IdErrorKind),
    /// {0:?}
    #[error(transparent)]
    FromIgnoreError(#[from] IgnoreErrorKind),
    /// {0:?}
    #[error(transparent)]
    FromBackendDecryptionError(#[from] CryptBackendErrorKind),
    /// generic Ignore error: `{0:?}`
    GenericError(#[from] ignore::Error),
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
}

/// [`ConfigFileErrorKind`] describes the errors that can be returned for `ConfigFile`s
#[derive(Error, Debug, Display)]
pub enum ConfigFileErrorKind {
    /// config version not supported!
    ConfigVersionNotSupported,
    /// Parsing Polynomial in config failed: `{0:?}`
    ParsingFailedForPolynomial(#[from] ParseIntError),
}

/// [`KeyFileErrorKind`] describes the errors that can be returned for `KeyFile`s
#[derive(Error, Debug, Display)]
pub enum KeyFileErrorKind {
    /// no suitable key found!
    NoSuitableKeyFound,
    /// listing KeyFiles failed
    ListingKeyFilesFailed,
    /// couldn't get KeyFile from backend
    CouldNotGetKeyFileFromBackend,
    /// serde_json couldn't deserialize the data: `{0:?}`
    DeserializingFromSliceFailed(serde_json::Error),
    /// couldn't encrypt data: `{0:?}`
    CouldNotEncryptData(#[from] CryptoErrorKind),
    /// serde_json couldn't serialize the data into a JSON byte vector: `{0:?}`
    CouldNotSerializeAsJsonByteVector(serde_json::Error),
    /// conversion from `u32` to `u8` failed: `{0:?}`
    ConversionFromU32ToU8Failed(TryFromIntError),
    /// output length is invalid: `{0:?}`
    OutputLengthInvalid(scrypt::errors::InvalidOutputLen),
    /// invalid scrypt parameters
    InvalidSCryptParameters(scrypt::errors::InvalidParams),
}

/// [`PackFileErrorKind`] describes the errors that can be returned for `PackFile`s
#[derive(Error, Debug, Display)]
pub enum PackFileErrorKind {
    /// Failed reading binary representation of the pack header
    ReadingBinaryRepresentationFailed(binrw::Error),
    /// Failed writing binary representation of the pack header
    WritingBinaryRepresentationFailed(binrw::Error),
    /// Read header length is too large! Length: {size_real}, file size: {pack_size}
    HeaderLengthTooLarge { size_real: u32, pack_size: u32 },
    /// Read header length doesn't match header contents! Length: {size_real}, computed: {size_computed}
    HeaderLengthDoesNotMatchHeaderContents { size_real: u32, size_computed: u32 },
    /// pack size computed from header doesn't match real pack isch! Computed: {size_computed}, real: {size_real}
    HeaderPackSizeComputedDoesNotMatchRealPackFile { size_real: u32, size_computed: u32 },
    /// partially reading the pack header from packfile failed: `{0:?}`
    ListingKeyFilesFailed(#[from] BackendAccessErrorKind),
    /// decrypting from binary failed
    BinaryDecryptionFailed,
    /// Partial read of PackFile failed
    PartialReadOfPackfileFailed,
    /// writing Bytes failed
    WritingBytesFailed,
    /// decryption on backend failed: `{0:?}`
    PackDecryptionFailed(#[from] CryptBackendErrorKind),
}

/// [`SnapshotFileErrorKind`] describes the errors that can be returned for `SnapshotFile`s
#[derive(Error, Debug, Display)]
pub enum SnapshotFileErrorKind {
    /// non-unicode hostname {0:?}
    NonUnicodeHostname(OsString),
    /// non-unicode path {0:?}
    NonUnicodePath(PathBuf),
    /// no snapshots found
    NoSnapshotsFound,
    /// value {0:?} not allowed
    ValueNotAllowed(String),
    /// datetime out of range: `{0:?}`
    OutOfRange(#[from] OutOfRangeError),
    /// reading the description file failed: `{0:?}`
    ReadingDescriptionFailed(#[from] std::io::Error),
    /// getting the SnapshotFile from the backend failed
    GettingSnapshotFileFailed,
    /// getting the SnapshotFile by ID failed
    GettingSnapshotFileByIdFailed,
    /// unpacking SnapshotFile result failed
    UnpackingSnapshotFileResultFailed,
    /// collecting IDs failed: {0:?}
    FindingIdsFailed(Vec<String>),
    /// removing dots from paths failed: `{0:?}`
    RemovingDotsFromPathFailed(std::io::Error),
    /// canonicalizing path failed: `{0:?}`
    CanonicalizingPathFailed(std::io::Error),
}

/// [`PackerErrorKind`] describes the errors that can be returned for a Packer
#[derive(Error, Debug, Display)]
pub enum PackerErrorKind {
    /// error returned by cryptographic libraries: `{0:?}`
    CryptoError(#[from] CryptoErrorKind),
    /// could not compress due to unsupported config version: `{0:?}`
    ConfigVersionNotSupported(#[from] ConfigFileErrorKind),
    /// compressing data failed: `{0:?}`
    CompressingDataFailed(#[from] std::io::Error),
    /// getting total size failed
    GettingTotalSizeFailed,
    /// crossbeam couldn't send message: `{0:?}`
    SendingCrossbeamMessageFailed(
        #[from] crossbeam_channel::SendError<(bytes::Bytes, BlobId, Option<u32>)>,
    ),
    /// crossbeam couldn't send message: `{0:?}`
    SendingCrossbeamMessageFailedForIndexPack(
        #[from] crossbeam_channel::SendError<(bytes::Bytes, IndexPack)>,
    ),
    /// couldn't create binary representation for pack header: `{0:?}`
    CouldNotCreateBinaryRepresentationForHeader(#[from] PackFileErrorKind),
    /// failed to write bytes in backend: `{0:?}`
    WritingBytesFailedInBackend(#[from] BackendAccessErrorKind),
    /// failed to write bytes for PackFile: `{0:?}`
    WritingBytesFailedForPackFile(PackFileErrorKind),
    /// failed to read partially encrypted data: `{0:?}`
    ReadingPartiallyEncryptedDataFailed(#[from] CryptBackendErrorKind),
    /// failed to partially read  data: `{0:?}`
    PartiallyReadingDataFailed(PackFileErrorKind),
    /// failed to add index pack: {0:?}
    AddingIndexPackFailed(#[from] IndexErrorKind),
    /// conversion for integer failed: `{0:?}`
    IntConversionFailed(#[from] TryFromIntError),
}

/// [`TreeErrorKind`] describes the errors that can come up dealing with Trees
#[derive(Error, Debug, Display)]
pub enum TreeErrorKind {
    /// blob {0:?} not found in index
    BlobIdNotFound(TreeId),
    /// {0:?} is no dir
    NotADirectory(OsString),
    /// "{0:?} not found"
    PathNotFound(OsString),
    /// path should not contain current or parent dir
    ContainsCurrentOrParentDirectory,
    /// serde_json couldn't serialize the tree: `{0:?}`
    SerializingTreeFailed(#[from] serde_json::Error),
    /// serde_json couldn't deserialize tree from bytes of JSON text: {0:?}
    DeserializingTreeFailed(serde_json::Error),
    /// reading blob data failed `{0:?}`
    ReadingBlobDataFailed(#[from] IndexErrorKind),
    /// slice is not UTF-8: {0:?}
    PathIsNotUtf8Conform(#[from] Utf8Error),
    /// error in building nodestreamer: `{0:?}`
    BuildingNodeStreamerFailed(#[from] ignore::Error),
    /// failed to read file string from glob file: `{0:?}`
    ReadingFileStringFromGlobsFailed(#[from] std::io::Error),
    /// crossbeam couldn't send message: `{0:?}`
    SendingCrossbeamMessageFailed(#[from] crossbeam_channel::SendError<(PathBuf, TreeId, usize)>),
    /// crossbeam couldn't receive message: `{0:?}`
    ReceivingCrossbreamMessageFailed(#[from] crossbeam_channel::RecvError),
}

/// [`CacheBackendErrorKind`] describes the errors that can be returned by a Caching action in Backends
#[derive(Error, Debug, Display)]
pub enum CacheBackendErrorKind {
    /// no cache dir
    NoCacheDirectory,
    /// `{0:?}`
    #[error(transparent)]
    FromIoError(#[from] std::io::Error),
    /// setting option on CacheBackend failed
    SettingOptionOnCacheBackendFailed,
    /// listing with size on CacheBackend failed
    ListingWithSizeOnCacheBackendFailed,
    /// fully reading from CacheBackend failed
    FullyReadingFromCacheBackendFailed,
    /// partially reading from CacheBackend failed
    PartiallyReadingFromBackendDataFailed,
    /// creating data on CacheBackend failed
    CreatingDataOnCacheBackendFailed,
    /// writing bytes on CacheBackend failed
    WritingBytesOnCacheBackendFailed,
    /// removing data on CacheBackend failed
    RemovingDataOnCacheBackendFailed,
}

/// [`CryptBackendErrorKind`] describes the errors that can be returned by a Decryption action in Backends
#[derive(Error, Debug, Display)]
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
    /// `{0:?}`
    #[error(transparent)]
    FromKey(#[from] CryptoErrorKind),
    /// `{0:?}`
    #[error(transparent)]
    FromIo(#[from] std::io::Error),
    /// `{0:?}`
    #[error(transparent)]
    FromJson(#[from] serde_json::Error),
    /// writing full hash failed in CryptBackend
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
    IntConversionFailed(#[from] TryFromIntError),
    /// Extra verification failed: After decrypting and decompressing the data changed!
    ExtraVerificationFailed,
}

/// [`IgnoreErrorKind`] describes the errors that can be returned by a Ignore action in Backends
#[derive(Error, Debug, Display)]
pub enum IgnoreErrorKind {
    /// generic Ignore error: `{0:?}`
    GenericError(#[from] ignore::Error),
    /// Error reading glob file {file:?}: {source:?}
    ErrorGlob {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Unable to open file {file:?}: {source:?}
    UnableToOpenFile {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Error getting xattrs for {path:?}: {source:?}
    ErrorXattr {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Error reading link target for {path:?}: {source:?}
    ErrorLink {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `{0:?}`
    #[error(transparent)]
    FromTryFromIntError(#[from] TryFromIntError),
}

/// [`LocalDestinationErrorKind`] describes the errors that can be returned by an action on the filesystem in Backends
#[derive(Error, Debug, Display)]
pub enum LocalDestinationErrorKind {
    /// directory creation failed: `{0:?}`
    DirectoryCreationFailed(#[from] std::io::Error),
    /// file `{0:?}` should have a parent
    FileDoesNotHaveParent(PathBuf),
    /// {0:?}
    #[error(transparent)]
    FromTryIntError(#[from] TryFromIntError),
    /// {0:?}
    #[error(transparent)]
    FromIdError(#[from] IdErrorKind),
    /// {0:?}
    #[error(transparent)]
    FromWalkdirError(#[from] walkdir::Error),
    /// {0:?}#
    #[error(transparent)]
    #[cfg(not(windows))]
    FromErrnoError(#[from] Errno),
    /// listing xattrs on {path:?}: {source:?}
    #[cfg(not(any(windows, target_os = "openbsd")))]
    ListingXattrsFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// setting xattr {name} on {filename:?} with {source:?}
    #[cfg(not(any(windows, target_os = "openbsd")))]
    SettingXattrFailed {
        name: String,
        filename: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// getting xattr {name} on {filename:?} with {source:?}
    #[cfg(not(any(windows, target_os = "openbsd")))]
    GettingXattrFailed {
        name: String,
        filename: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// removing directories failed: `{0:?}`
    DirectoryRemovalFailed(std::io::Error),
    /// removing file failed: `{0:?}`
    FileRemovalFailed(std::io::Error),
    /// setting time metadata failed: `{0:?}`
    SettingTimeMetadataFailed(std::io::Error),
    /// opening file failed: `{0:?}`
    OpeningFileFailed(std::io::Error),
    /// setting file length failed: `{0:?}`
    SettingFileLengthFailed(std::io::Error),
    /// can't jump to position in file: `{0:?}`
    CouldNotSeekToPositionInFile(std::io::Error),
    /// couldn't write to buffer: `{0:?}`
    CouldNotWriteToBuffer(std::io::Error),
    /// reading exact length of file contents failed: `{0:?}`
    ReadingExactLengthOfFileFailed(std::io::Error),
    /// setting file permissions failed: `{0:?}`
    #[cfg(not(windows))]
    SettingFilePermissionsFailed(std::io::Error),
    /// failed to symlink target {linktarget:?} from {filename:?} with {source:?}
    #[cfg(not(windows))]
    SymlinkingFailed {
        linktarget: PathBuf,
        filename: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// [`NodeErrorKind`] describes the errors that can be returned by an action utilizing a node in Backends
#[derive(Error, Debug, Display)]
pub enum NodeErrorKind {
    /// {0:?}
    FromParseIntError(#[from] ParseIntError),
    /// Unexpected EOF
    #[cfg(not(windows))]
    UnexpectedEOF,
    /// Invalid unicode
    #[cfg(not(windows))]
    InvalidUnicode,
    /// Unrecognized Escape
    #[cfg(not(windows))]
    UnrecognizedEscape,
}

/// [`StdInErrorKind`] describes the errors that can be returned while dealing IO from CLI
#[derive(Error, Debug, Display)]
pub enum StdInErrorKind {
    /// StdIn Error: `{0:?}`
    StdInError(#[from] std::io::Error),
}

/// [`ArchiverErrorKind`] describes the errors that can be returned from the archiver
#[derive(Error, Debug, Display)]
pub enum ArchiverErrorKind {
    /// tree stack empty
    TreeStackEmpty,
    /// cannot open file
    OpeningFileFailed,
    /// option should contain a value, but contained `None`
    UnpackingTreeTypeOptionalFailed,
    /// couldn't get size for archive: `{0:?}`
    CouldNotGetSizeForArchive(#[from] BackendAccessErrorKind),
    /// couldn't determine size for item in Archiver
    CouldNotDetermineSize,
    /// failed to save index: `{0:?}`
    IndexSavingFailed(#[from] IndexErrorKind),
    /// failed to save file in backend: `{0:?}`
    FailedToSaveFileInBackend(#[from] CryptBackendErrorKind),
    /// finalizing SnapshotSummary failed: `{0:?}`
    FinalizingSnapshotSummaryFailed(#[from] SnapshotFileErrorKind),
    /// `{0:?}`
    #[error(transparent)]
    FromPacker(#[from] PackerErrorKind),
    /// `{0:?}`
    #[error(transparent)]
    FromTree(#[from] TreeErrorKind),
    /// `{0:?}`
    #[error(transparent)]
    FromConfigFile(#[from] ConfigFileErrorKind),
    /// `{0:?}`
    #[error(transparent)]
    FromStdIo(#[from] std::io::Error),
    /// `{0:?}`
    #[error(transparent)]
    FromStripPrefix(#[from] StripPrefixError),
    /// conversion from `u64` to `usize` failed: `{0:?}`
    ConversionFromU64ToUsizeFailed(TryFromIntError),
}

/// [`VfsErrorKind`] describes the errors that can be returned from the Virtual File System
#[derive(Error, Debug, Display)]
pub enum VfsErrorKind {
    /// No directory entries for symlink found: `{0:?}`
    NoDirectoryEntriesForSymlinkFound(OsString),
    /// Directory exists as non-virtual directory
    DirectoryExistsAsNonVirtual,
    /// Only normal paths allowed
    OnlyNormalPathsAreAllowed,
    /// Name `{0:?}`` doesn't exist
    NameDoesNotExist(OsString),
}

trait RusticErrorMarker: Error {}

impl RusticErrorMarker for CryptoErrorKind {}
impl RusticErrorMarker for PolynomialErrorKind {}
impl RusticErrorMarker for IdErrorKind {}
impl RusticErrorMarker for RepositoryErrorKind {}
impl RusticErrorMarker for IndexErrorKind {}
impl RusticErrorMarker for BackendAccessErrorKind {}
impl RusticErrorMarker for ConfigFileErrorKind {}
impl RusticErrorMarker for KeyFileErrorKind {}
impl RusticErrorMarker for PackFileErrorKind {}
impl RusticErrorMarker for SnapshotFileErrorKind {}
impl RusticErrorMarker for PackerErrorKind {}
impl RusticErrorMarker for FileErrorKind {}
impl RusticErrorMarker for TreeErrorKind {}
impl RusticErrorMarker for CacheBackendErrorKind {}
impl RusticErrorMarker for CryptBackendErrorKind {}
impl RusticErrorMarker for IgnoreErrorKind {}
impl RusticErrorMarker for LocalDestinationErrorKind {}
impl RusticErrorMarker for NodeErrorKind {}
impl RusticErrorMarker for StdInErrorKind {}
impl RusticErrorMarker for ArchiverErrorKind {}
impl RusticErrorMarker for CommandErrorKind {}
impl RusticErrorMarker for VfsErrorKind {}
impl RusticErrorMarker for std::io::Error {}

impl<E> From<E> for RusticError
where
    E: RusticErrorMarker,
    RusticErrorKind: From<E>,
{
    fn from(value: E) -> Self {
        Self(RusticErrorKind::from(value))
    }
}
