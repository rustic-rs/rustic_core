//! The commands that can be run by the CLI.

use std::{num::TryFromIntError, path::PathBuf};

use chrono::OutOfRangeError;

use crate::{backend::node::NodeType, blob::BlobId, repofile::packfile::PackId, RusticError};

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
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
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
    /// Bytesize parser failed: `{0}`
    FromByteSizeParser(String),
    /// --repack-uncompressed makes no sense for v1 repo!
    RepackUncompressedRepoV1,
    /// datetime out of range: `{0}`
    FromOutOfRangeError(OutOfRangeError),
    /// node type `{0:?}` not supported by dump
    DumpNotSupported(NodeType),
    /// error creating `{0:?}`: `{1:?}`
    ErrorCreating(PathBuf, Box<RusticError>),
    /// error collecting information for `{0:?}`: `{1:?}`
    ErrorCollecting(PathBuf, Box<RusticError>),
    /// error setting length for `{0:?}`: `{1:?}`
    ErrorSettingLength(PathBuf, Box<RusticError>),
    /// Conversion from integer failed: `{0:?}`
    ConversionFromIntFailed(TryFromIntError),
    /// Specify one of the keep-* options for forget! Please use keep-none to keep no snapshot.
    NoKeepOption,
    /// Checking the repository failed!
    CheckFailed,
}

pub(crate) type CommandResult<T> = Result<T, CommandErrorKind>;
