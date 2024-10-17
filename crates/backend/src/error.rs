#![allow(clippy::doc_markdown)]
use std::{num::TryFromIntError, process::ExitStatus, str::Utf8Error};

use displaydoc::Display;
use thiserror::Error;

/// [`BackendAccessErrorKind`] describes the errors that can be returned by the various Backends
#[derive(Error, Debug, Display)]
#[non_exhaustive]
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
