//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use derive_setters::Setters;
use std::fmt::{self, Display};
use thiserror::Error;

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T> = Result<T, RusticError>;

#[derive(Error, Debug, Setters)]
#[setters(strip_option)]
/// Errors that can result from rustic.
pub struct RusticError {
    /// The source of the error.
    // TODO! We should think if it makes sense to erase the type here, or if we should
    // TODO! rather use RusticErrorKind here and create some higher level errors there,
    // TODO! that are most needed for the user. E.g. something like `IncorrectPassword`
    // TODO! or general failures for the repository.
    source: Box<dyn std::error::Error>,

    /// The message of the error.
    message: Option<String>,

    /// The URL of the documentation for the error.
    docs_url: Option<String>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<String>,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_url: Option<String>,
}

impl Display for RusticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "An error occurred in `rustic_core`: ")?;

        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }

        write!(f, "{}", self.source)?;

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
            write!(f, "\n\nThis might be a related issue, please check it for a possible workaround and/or further guidance: {}", existing_issue_url)?;
        }

        Ok(())
    }
}

// Accessors for anything we do want to expose publicly.
impl RusticError {
    pub fn new<T: std::error::Error + Display + 'static>(source: T) -> Self {
        Self {
            source: Box::new(source),
            message: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
        }
    }

    /// Expose the inner error kind.
    ///
    /// This is useful for matching on the error kind.
    pub fn into_inner(self) -> RusticErrorKind {
        todo!()
    }

    /// Checks if the error is due to an incorrect password
    pub fn is_incorrect_password(&self) -> bool {
        matches!(
            self.source,
            RusticErrorKind::Repository(RepositoryErrorKind::IncorrectPassword)
        )
    }

    /// Get the corresponding backend error, if error is caused by the backend.
    ///
    /// Returns `anyhow::Error`; you need to cast this to the real backend error type
    pub fn backend_error(&self) -> Option<&anyhow::Error> {
        if let RusticErrorKind::Backend(error) = &self.source {
            Some(error)
        } else {
            None
        }
    }

    pub fn from<T: std::error::Error + Display + 'static>(error: T) -> Self {
        Self {
            message: error.to_string().into(),
            source: Box::new(error),
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
        }
    }
}

/// [`RusticErrorKind`] describes the errors that can happen while executing a high-level command.
///
/// This is a non-exhaustive enum, so additional variants may be added in future. It is
/// recommended to match against the wildcard `_` instead of listing all possible variants,
/// to avoid problems when new variants are added.
#[non_exhaustive]
#[derive(Error, Debug, displaydoc::Display)]
pub enum RusticErrorKind {
    /// Describes the errors that can be returned by the various backends from the `rustic_backend` crate.
    #[error(transparent)]
    Backend(#[from] anyhow::Error),
    /// [`std::io::Error`]
    #[error(transparent)]
    FromIo(#[from] std::io::Error),
}
