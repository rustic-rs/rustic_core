//! The commands that can be run by the CLI.

use std::{
    num::{ParseFloatError, ParseIntError, TryFromIntError},
    ops::RangeInclusive,
    path::PathBuf,
};

use chrono::OutOfRangeError;
use displaydoc::Display;
use thiserror::Error;

use crate::{backend::node::NodeType, blob::BlobId, repofile::packfile::PackId};

pub mod backup;
/// The `cat` command.
pub mod cat;
pub mod check;
pub mod config;
/// The `copy` command.
pub mod copy;
/// The `dump` command.
pub mod dump;
pub mod forget;
pub mod init;
pub mod key;
pub mod merge;
pub mod prune;
/// The `repair` command.
pub mod repair;
/// The `repoinfo` command.
pub mod repoinfo;
pub mod restore;
pub mod snapshots;

/// [`CommandErrorKind`] describes the errors that can happen while executing a high-level command
#[derive(Error, Debug, Display)]
pub enum CommandErrorKind {
    /// path is no dir: `{0}`
    PathIsNoDir(String),
    /// used blobs are missing: blob `{0}` doesn't existing
    BlobsMissing(BlobId),
    /// used pack `{0}`: size does not match! Expected size: `{1}`, real size: `{2}`
    PackSizeNotMatching(PackId, u32, u32),
    /// used pack `{0}` does not exist!
    PackNotExisting(PackId),
    /// pack `{0}` got no decision what to do
    NoDecision(PackId),
    /// [`std::num::ParseFloatError`]
    #[error(transparent)]
    FromParseFloatError(#[from] ParseFloatError),
    /// [`std::num::ParseIntError`]
    #[error(transparent)]
    FromParseIntError(#[from] ParseIntError),
    /// Bytesize parser failed: `{0}`
    FromByteSizeParser(String),
    /// --repack-uncompressed makes no sense for v1 repo!
    RepackUncompressedRepoV1,
    /// datetime out of range: `{0}`
    FromOutOfRangeError(#[from] OutOfRangeError),
    /// node type `{0:?}` not supported by dump
    DumpNotSupported(NodeType),
    /// [`serde_json::Error`]
    #[error(transparent)]
    FromJsonError(#[from] serde_json::Error),
    /// version `{0}` is not supported. Allowed values: {1:?}
    VersionNotSupported(u32, RangeInclusive<u32>),
    /// cannot downgrade version from `{0}` to `{1}`
    CannotDowngrade(u32, u32),
    /// compression level `{0}` is not supported for repo v1
    NoCompressionV1Repo(i32),
    /// compression level `{0}` is not supported. Allowed values: `{1:?}`
    CompressionLevelNotSupported(i32, RangeInclusive<i32>),
    /// Size is too large: `{0}`
    SizeTooLarge(bytesize::ByteSize),
    /// min_packsize_tolerate_percent must be <= 100
    MinPackSizeTolerateWrong,
    /// max_packsize_tolerate_percent must be >= 100 or 0"
    MaxPackSizeTolerateWrong,
    /// error creating `{0:?}`: `{1:?}`
    ErrorCreating(PathBuf, Box<RusticError>),
    /// error collecting information for `{0:?}`: `{1:?}`
    ErrorCollecting(PathBuf, Box<RusticError>),
    /// error setting length for `{0:?}`: `{1:?}`
    ErrorSettingLength(PathBuf, Box<RusticError>),
    /// [`rayon::ThreadPoolBuildError`]
    #[error(transparent)]
    FromRayonError(#[from] rayon::ThreadPoolBuildError),
    /// Conversion from integer failed: `{0:?}`
    ConversionFromIntFailed(TryFromIntError),
    /// Not allowed on an append-only repository: `{0}`
    NotAllowedWithAppendOnly(String),
    /// Specify one of the keep-* options for forget! Please use keep-none to keep no snapshot.
    NoKeepOption,
    /// [`shell_words::ParseError`]
    #[error(transparent)]
    FromParseError(#[from] shell_words::ParseError),
    /// Checking the repository failed!
    CheckFailed,
}

pub(crate) type CommandResult<T> = Result<T, CommandErrorKind>;
