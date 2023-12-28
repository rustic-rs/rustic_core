use std::{
    num::{ParseIntError, TryFromIntError},
    process::ExitStatus,
    str::Utf8Error,
};

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
    /// backoff failed: {0:?}
    BackoffError(#[from] backoff::Error<reqwest::Error>),
    /// parsing failed for url: `{0:?}`
    UrlParsingFailed(#[from] url::ParseError),
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
    /// `{0:?}`
    #[error(transparent)]
    FromParseIntError(#[from] ParseIntError),
}

/// [`RestErrorKind`] describes the errors that can be returned while dealing with the REST API
#[derive(Error, Debug, Display)]
pub enum RestErrorKind {
    /// value `{0:?}` not supported for option retry!
    NotSupportedForRetry(String),
    /// parsing failed for url: `{0:?}`
    UrlParsingFailed(#[from] url::ParseError),
    /// requesting resource failed: `{0:?}`
    RequestingResourceFailed(#[from] reqwest::Error),
    /// couldn't parse duration in humantime library: `{0:?}`
    CouldNotParseDuration(#[from] humantime::DurationError),
    /// backoff failed: {0:?}
    BackoffError(#[from] backoff::Error<reqwest::Error>),
    /// Failed to build HTTP client: `{0:?}`
    BuildingClientFailed(reqwest::Error),
    /// joining URL failed on: {0:?}
    JoiningUrlFailed(url::ParseError),
}
