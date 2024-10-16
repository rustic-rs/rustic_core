//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use std::{
    ffi::OsString,
    num::{ParseIntError, TryFromIntError},
    path::PathBuf,
    process::ExitStatus,
    str::Utf8Error,
};

#[cfg(not(windows))]
use nix::errno::Errno;

use chrono::OutOfRangeError;
use displaydoc::Display;
use thiserror::Error;

use crate::{
    blob::{tree::TreeId, BlobId},
    id::Id,
    repofile::{indexfile::IndexPack, packfile::PackId, BlobType},
    FileType,
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
pub enum RusticErrorKind {}
