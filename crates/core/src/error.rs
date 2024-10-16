//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use std::fmt::{self, Display};

use displaydoc::Display;
use thiserror::Error;

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T> = Result<T, RusticError>;

#[derive(Error, Debug)]
/// Errors that can result from rustic.
pub struct RusticError {
    /// The kind of error.
    kind: RusticErrorKind,

    /// The message of the error.
    message: Option<String>,

    /// The URL of the documentation for the error.
    docs_url: Option<String>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<String>,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_url: Option<String>,

    /// The backtrace of the error.
    backtrace: Option<std::backtrace::Backtrace>,
}

impl Display for RusticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "An error occurred in `rustic_core`: ")?;

        write!(f, "{}", self.kind)?;

        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }

        if let Some(docs_url) = &self.docs_url {
            write!(f, "\n\nFor more information, see: {}", docs_url)?;
        }

        if let Some(new_issue_url) = &self.new_issue_url {
            write!(
                f,
                "\n\nIf you think this is a bug, please open an issue at: {}",
                new_issue_url
            )?;
        }

        if let Some(existing_issue_url) = &self.existing_issue_url {
            write!(f, "\n\nA related issue might be, please check it for a possible workaround and/or guidance: {}", existing_issue_url)?;
        }

        if let Some(backtrace) = &self.backtrace {
            write!(f, "\n\nBacktrace:\n{:?}", backtrace)?;
        }

        Ok(())
    }
}

// Accessors for anything we do want to expose publicly.
impl RusticError {
    /// Expose the inner error kind.
    ///
    /// This is useful for matching on the error kind.
    pub fn into_inner(self) -> RusticErrorKind {
        self.kind
    }

    /// Checks if the error is due to an incorrect password
    pub fn is_incorrect_password(&self) -> bool {
        matches!(
            self.kind,
            RusticErrorKind::Repository(RepositoryErrorKind::IncorrectPassword)
        )
    }

    /// Get the corresponding backend error, if error is caused by the backend.
    ///
    /// Returns `anyhow::Error`; you need to cast this to the real backend error type
    pub fn backend_error(&self) -> Option<&anyhow::Error> {
        if let RusticErrorKind::Backend(error) = &self.kind {
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
    /// Describes the errors that can be returned by the various backends from the `rustic_backend` crate.
    #[error(transparent)]
    Backend(#[from] anyhow::Error),
}
