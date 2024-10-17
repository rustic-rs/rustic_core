//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use binrw::docs;
use derive_setters::Setters;
use std::{
    default,
    fmt::{self, Display},
};
use thiserror::Error;

use crate::error::immut_str::ImmutStr;

pub(crate) mod constants {
    pub const DEFAULT_DOCS_URL: &str = "https://rustic.cli.rs/docs/errors/";
    pub const DEFAULT_ISSUE_URL: &str = "https://github.com/rustic-rs/rustic_core/issues/new";
}

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T> = Result<T, RusticError>;

#[derive(Error, Debug, Setters, Default)]
#[setters(strip_option)]
/// Errors that can result from rustic.
pub struct RusticError {
    /// The kind of the error.
    kind: RusticErrorKind,

    /// Chain to the cause of the error.
    cause: Option<Box<(dyn std::error::Error + Send + Sync)>>,

    /// The context of the error.
    context: Option<ImmutStr>,

    /// The URL of the documentation for the error.
    docs_url: Option<ImmutStr>,

    /// Error code.
    code: Option<ImmutStr>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<ImmutStr>,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_url: Option<ImmutStr>,

    /// Severity of the error.
    severity: Option<ErrorSeverity>,
}

impl Display for RusticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "An error occurred in `rustic_core`: {}", self.kind)?;

        if let Some(context) = &self.context {
            write!(f, "\n\n => {context}")?;
        }

        if let Some(cause) = &self.cause {
            write!(f, "\n\nCaused by: {cause}")?;
        }

        if let Some(code) = &self.code {
            let docs_url = self
                .docs_url
                .as_ref()
                .unwrap_or(&ImmutStr::from(constants::DEFAULT_DOCS_URL));

            write!(f, "\n\nFor more information, see: {docs_url}/{code}")?;
        }

        if let Some(existing_issue_url) = &self.existing_issue_url {
            write!(f, "\n\nThis might be a related issue, please check it for a possible workaround and/or further guidance: {existing_issue_url}")?;
        }

        let new_issue_url = self
            .new_issue_url
            .as_ref()
            .unwrap_or(&ImmutStr::from(constants::DEFAULT_ISSUE_URL));

        write!(
            f,
            "\n\nIf you think this is an undiscovered bug, please open an issue at: {new_issue_url}"
        )?;

        Ok(())
    }
}

// Accessors for anything we do want to expose publicly.
impl RusticError {
    pub fn new(kind: RusticErrorKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }

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
        if let RusticErrorKind::Backend(error) = &self.cause {
            Some(error)
        } else {
            None
        }
    }

    pub fn from<T: std::error::Error + Display + Send + Sync + 'static>(
        error: T,
        kind: RusticErrorKind,
    ) -> Self {
        Self {
            kind,
            context: Some(error.to_string().into()),
            cause: Some(Box::new(error)),
            code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorSeverity {
    #[default]
    Info,
    Warning,
    Error,
    Fatal,
}

/// [`RusticErrorKind`] describes the errors that can happen while executing a high-level command.
///
/// This is a non-exhaustive enum, so additional variants may be added in future. It is
/// recommended to match against the wildcard `_` instead of listing all possible variants,
/// to avoid problems when new variants are added.
#[non_exhaustive]
#[derive(Error, Debug, displaydoc::Display, Default)]
pub enum RusticErrorKind {
    /// None
    // This is a placeholder variant to avoid having to use `Option` in the `RusticError` struct.
    #[default]
    None,
    /// Describes the errors that can be returned by the various backends from the `rustic_backend` crate.
    #[error(transparent)]
    Backend(#[from] anyhow::Error),
    /// [`std::io::Error`]
    #[error(transparent)]
    FromIo(#[from] std::io::Error),
}

pub mod immut_str {
    //! Copyright 2024 Cloudflare, Inc.
    //!
    //! Licensed under the Apache License, Version 2.0 (the "License");
    //! you may not use this file except in compliance with the License.
    //! You may obtain a copy of the License at
    //!
    //! http://www.apache.org/licenses/LICENSE-2.0
    //!
    //! Unless required by applicable law or agreed to in writing, software
    //! distributed under the License is distributed on an "AS IS" BASIS,
    //! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    //! See the License for the specific language governing permissions and
    //! limitations under the License.
    //!
    //! Taken from <https://github.com/cloudflare/pingora/blob/51516839f7155dd74d5cf93006ec1df9ea126b11/pingora-error/src/immut_str.rs>

    use std::fmt;

    /// A data struct that holds either immutable string or reference to static str.
    /// Compared to String or `Box<str>`, it avoids memory allocation on static str.
    #[derive(Debug, PartialEq, Eq, Clone)]
    pub enum ImmutStr {
        Static(&'static str),
        Owned(Box<str>),
    }

    impl ImmutStr {
        #[inline]
        pub fn as_str(&self) -> &str {
            match self {
                ImmutStr::Static(s) => s,
                ImmutStr::Owned(s) => s.as_ref(),
            }
        }

        pub fn is_owned(&self) -> bool {
            match self {
                ImmutStr::Static(_) => false,
                ImmutStr::Owned(_) => true,
            }
        }
    }

    impl fmt::Display for ImmutStr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.as_str())
        }
    }

    impl From<&'static str> for ImmutStr {
        fn from(s: &'static str) -> Self {
            ImmutStr::Static(s)
        }
    }

    impl From<String> for ImmutStr {
        fn from(s: String) -> Self {
            ImmutStr::Owned(s.into_boxed_str())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_static_vs_owned() {
            let s: ImmutStr = "test".into();
            assert!(!s.is_owned());
            let s: ImmutStr = "test".to_string().into();
            assert!(s.is_owned());
        }
    }
}
