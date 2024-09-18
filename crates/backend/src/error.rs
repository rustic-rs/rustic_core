#![allow(clippy::doc_markdown)]
use std::{num::TryFromIntError, process::ExitStatus, str::Utf8Error};

use displaydoc::Display;
use thiserror::Error;

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
    #[cfg(feature = "rest")]
    /// backoff failed: {0:?}
    BackoffError(#[from] backoff::Error<reqwest::Error>),
    /// parsing failed for url: `{0:?}`
    UrlParsingFailed(#[from] url::ParseError),
    /// creating data in backend failed
    CreatingDataOnBackendFailed,
    /// writing bytes to backend failed
    WritingBytesToBackendFailed,
    /// removing data from backend failed
    RemovingDataFromBackendFailed,
    /// failed to list files on Backend
    ListingFilesOnBackendFailed,
}

/// [`RcloneErrorKind`] describes the errors that can be returned by a backend provider
#[derive(Error, Debug, Display)]
pub enum RcloneErrorKind {
    /// 'rclone version' doesn't give any output
    NoOutputForRcloneVersion,
    /// cannot get stdout of rclone
    NoStdOutForRclone,
    /// rclone exited with `{0:?}`
    RCloneExitWithBadStatus(ExitStatus),
    /// url must start with http:\/\/! url: {0:?}
    UrlNotStartingWithHttp(String),
    /// StdIo Error: `{0:?}`
    #[error(transparent)]
    FromIoError(#[from] std::io::Error),
    /// utf8 error: `{0:?}`
    #[error(transparent)]
    FromUtf8Error(#[from] Utf8Error),
    /// error parsing verision number from `{0:?}`
    FromParseVersion(String),
    /// Using rclone without authentication! Upgrade to rclone >= 1.52.2 (current version: `{0}`)!
    RCloneWithoutAuthentication(String),
}

/// [`RestErrorKind`] describes the errors that can be returned while dealing with the REST API
#[derive(Error, Debug, Display)]
pub enum RestErrorKind {
    /// value `{0:?}` not supported for option retry!
    NotSupportedForRetry(String),
    /// parsing failed for url: `{0:?}`
    UrlParsingFailed(#[from] url::ParseError),
    #[cfg(feature = "rest")]
    /// requesting resource failed: `{0:?}`
    RequestingResourceFailed(#[from] reqwest::Error),
    /// couldn't parse duration in humantime library: `{0:?}`
    CouldNotParseDuration(#[from] humantime::DurationError),
    #[cfg(feature = "rest")]
    /// backoff failed: {0:?}
    BackoffError(#[from] backoff::Error<reqwest::Error>),
    #[cfg(feature = "rest")]
    /// Failed to build HTTP client: `{0:?}`
    BuildingClientFailed(reqwest::Error),
    /// joining URL failed on: {0:?}
    JoiningUrlFailed(url::ParseError),
}

/// [`LocalBackendErrorKind`] describes the errors that can be returned by an action on the filesystem in Backends
#[derive(Error, Debug, Display)]
pub enum LocalBackendErrorKind {
    /// directory creation failed: `{0:?}`
    DirectoryCreationFailed(#[from] std::io::Error),
    /// querying metadata failed: `{0:?}`
    QueryingMetadataFailed(std::io::Error),
    /// querying WalkDir metadata failed: `{0:?}`
    QueryingWalkDirMetadataFailed(walkdir::Error),
    /// executtion of command failed: `{0:?}`
    CommandExecutionFailed(std::io::Error),
    /// command was not successful for filename {file_name}, type {file_type}, id {id}: {status}
    CommandNotSuccessful {
        /// File name
        file_name: String,
        /// File type
        file_type: String,
        /// Item ID
        id: String,
        /// Exit status
        status: ExitStatus,
    },
    /// error building automaton `{0:?}`
    FromAhoCorasick(#[from] aho_corasick::BuildError),
    /// {0:?}
    #[error(transparent)]
    FromTryIntError(#[from] TryFromIntError),
    /// {0:?}
    #[error(transparent)]
    FromWalkdirError(#[from] walkdir::Error),
    /// removing file failed: `{0:?}`
    FileRemovalFailed(std::io::Error),
    /// opening file failed: `{0:?}`
    OpeningFileFailed(std::io::Error),
    /// setting file length failed: `{0:?}`
    SettingFileLengthFailed(std::io::Error),
    /// can't jump to position in file: `{0:?}`
    CouldNotSeekToPositionInFile(std::io::Error),
    /// couldn't write to buffer: `{0:?}`
    CouldNotWriteToBuffer(std::io::Error),
    /// reading file contents failed: `{0:?}`
    ReadingContentsOfFileFailed(std::io::Error),
    /// reading exact length of file contents failed: `{0:?}`
    ReadingExactLengthOfFileFailed(std::io::Error),
    /// failed to sync OS Metadata to disk: `{0:?}`
    SyncingOfOsMetadataFailed(std::io::Error),
}
