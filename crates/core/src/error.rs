//! Error types and Result module.
//!
//! ## Error handling rules
//!
//! ### Visibility
//!
//! All `pub fn` (associated) functions need to return a `Result<T, RusticError> (==RusticResult<T>)`, if they are fallible.
//! As they are user facing and will cross the API boundary we need to make sure they are high-quality errors containing all
//! needed information and actionable guidance.
//!
//! `pub(crate) fn` visibility should use a local error and thus a Result and error type limited in visibility, e.g.
//! `pub(crate) type ArchiverResult<T> = Result<T, ArchiverErrorKind>`.
//!
//! ### Downgrading
//!
//! `RusticError`s should **not** be downgraded, instead we **upgrade** the function signature to contain a `RusticResult`.
//! For instance, if a function returns `Result<T, ArchiverErrorKind>` and we discover an error path that contains a `RusticError`,
//! we don't need to convert that into an `ArchiverErrorKind`, we should change the function signature, so it returns either a
//! `Result<T, RusticError> (==RusticResult<T>)` or nested results like `RusticResult<Result<T, ArchiverErrorKind>>`.
//! So even if the visibility of that function is `fn` or `pub(crate) fn` it should return a `RusticResult` containing a `RusticError`.
//!
//! ### Conversion and Nested Results
//!
//! Converting between different error kinds or their variants e.g. `TreeErrorKind::Channel` -> `ArchiverErrorKind::Channel`
//! should seldom happen (probably never?), as the caller is most likely not setup to handle such errors from a different layer,
//! so at this point, we should return either a `RusticError` indicating this is a hard error. Or use a nested Result, e.g.
//! `Result<Result<T, TreeErrorKind>, RusticError>`.
//!
//! Local error types in `pub fn` (associated) functions need to be manually converted into a `RusticError` with a good error message
//! and other important information, e.g. actionable guidance for the user.
//!
//! ### Backend traits
//!
//! By using `RusticResult` in our `Backend` traits, we also make sure, we get back presentable errors for our users.
//! We had them before as type erased errors, that we just bubbled up. Now we can provide more context and guidance.
//!
//! ### Traits
//!
//! All traits and implementations of (foreign) traits should use `RusticResult` as return type or `Box<RusticError>` as `Self::Err`.
//!
//! ### Display and Debug
//!
//! All types that we want to attach to an error should implement `Display` and `Debug` to provide a good error message and a nice way
//! to display the error.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use ::std::convert::Into;
use smol_str::SmolStr;
use std::{
    backtrace::Backtrace,
    borrow::Cow,
    fmt::{self, Display},
};

pub(crate) mod constants {
    pub const DEFAULT_DOCS_URL: &str = "https://rustic.cli.rs/docs/errors/";
    pub const DEFAULT_ISSUE_URL: &str = "https://github.com/rustic-rs/rustic_core/issues/new";
}

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T, E = Box<RusticError>> = Result<T, E>;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
/// Errors that can result from rustic.
pub struct RusticError {
    /// The kind of the error.
    kind: ErrorKind,

    /// Chain to the cause of the error.
    source: Option<Box<(dyn std::error::Error + Send + Sync)>>,

    /// The error message with guidance.
    guidance: SmolStr,

    /// The context of the error.
    context: Cow<'static, [(&'static str, SmolStr)]>,

    /// The URL of the documentation for the error.
    docs_url: Option<SmolStr>,

    /// Error code.
    error_code: Option<SmolStr>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<SmolStr>,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_url: Option<SmolStr>,

    /// Severity of the error.
    severity: Option<Severity>,

    /// The status of the error.
    status: Option<Status>,

    /// Backtrace of the error.
    ///
    // Need to use option, otherwise thiserror will not be able to derive the Error trait.
    backtrace: Option<Backtrace>,
}

impl Display for RusticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} occurred in `rustic_core`", self.kind)?;

        write!(f, "\n\nMessage:\n{}", self.guidance)?;

        if !self.context.is_empty() {
            write!(f, "\n\nContext:\n")?;
            write!(
                f,
                "{}",
                self.context
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(",\n")
            )?;
        }

        if let Some(cause) = &self.source {
            write!(f, "\n\nCaused by: {cause}")?;
        }

        if let Some(severity) = &self.severity {
            write!(f, "\n\nSeverity: {severity:?}")?;
        }

        if let Some(status) = &self.status {
            write!(f, "\n\nStatus: {status:?}")?;
        }

        if let Some(code) = &self.error_code {
            let default_docs_url = SmolStr::from(constants::DEFAULT_DOCS_URL);
            let docs_url = self.docs_url.as_ref().unwrap_or(&default_docs_url);

            write!(f, "\n\nFor more information, see: {docs_url}{code}")?;
        }

        if let Some(existing_issue_url) = &self.existing_issue_url {
            write!(f, "\n\nThis might be a related issue, please check it for a possible workaround and/or further guidance: {existing_issue_url}")?;
        }

        let default_issue_url = SmolStr::from(constants::DEFAULT_ISSUE_URL);
        let new_issue_url = self.new_issue_url.as_ref().unwrap_or(&default_issue_url);

        write!(
            f,
            "\n\nIf you think this is an undiscovered bug, please open an issue at: {new_issue_url}"
        )?;

        if let Some(backtrace) = &self.backtrace {
            write!(f, "\n\nBacktrace:\n{backtrace:?}")?;
        }

        Ok(())
    }
}

// Accessors for anything we do want to expose publicly.
impl RusticError {
    /// Creates a new error with the given kind and guidance.
    pub fn new(kind: ErrorKind, guidance: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: guidance.into(),
            context: Cow::default(),
            source: None,
            error_code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
            status: None,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        })
    }

    /// Creates a new error with the given kind and guidance.
    pub fn with_source(
        kind: ErrorKind,
        guidance: impl Into<String>,
        source: impl Into<Box<(dyn std::error::Error + Send + Sync)>>,
    ) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: guidance.into().into(),
            context: Cow::default(),
            source: Some(source.into()),
            error_code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
            status: None,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        })
    }

    /// Checks if the error has a specific error code.
    pub fn is_code(&self, code: &str) -> bool {
        self.error_code
            .as_ref()
            .map_or(false, |c| c.as_str() == code)
    }

    /// Checks if the error is due to an incorrect password
    pub fn is_incorrect_password(&self) -> bool {
        self.is_code("C002")
    }

    /// Creates a new error from a given error.
    pub fn from<T: std::error::Error + Display + Send + Sync + 'static>(
        error: T,
        kind: ErrorKind,
    ) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: error.to_string().into(),
            context: Cow::default(),
            source: Some(Box::new(error)),
            error_code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
            status: None,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        })
    }
}

// Setters for anything we do want to expose publicly.
//
// These were initially generated by `derive_setters`,
// and then manually adjusted to return `Box<Self>` instead of `Self` which
// unfortunately is not possible with the current version of the `derive_setters`.
//
// BEWARE! `attach_context` is manually implemented to allow for multiple contexts
// to be added and is not generated by `derive_setters`.
impl RusticError {
    /// Attach what kind the error is.
    pub fn overwrite_kind(self, value: impl Into<ErrorKind>) -> Box<Self> {
        Box::new(Self {
            kind: value.into(),
            ..self
        })
    }

    /// Attach a chain to the cause of the error.
    pub fn attach_source(
        self,
        value: impl Into<Box<(dyn std::error::Error + Send + Sync)>>,
    ) -> Box<Self> {
        Box::new(Self {
            source: Some(value.into()),
            ..self
        })
    }

    /// Attach the error message with guidance.
    pub fn overwrite_guidance(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            guidance: value.into(),
            ..self
        })
    }

    /// Append a newline to the guidance message.
    /// This is useful for adding additional information to the guidance.
    pub fn append_guidance_line(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            guidance: format!("{}\n{}", self.guidance, value.into()).into(),
            ..self
        })
    }

    /// Prepend a newline to the guidance message.
    /// This is useful for adding additional information to the guidance.
    pub fn prepend_guidance_line(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            guidance: format!("{}\n{}", value.into(), self.guidance).into(),
            ..self
        })
    }

    // IMPORTANT: This is manually implemented to allow for multiple contexts to be added.
    /// Attach context to the error.
    pub fn attach_context(mut self, key: &'static str, value: impl Into<SmolStr>) -> Box<Self> {
        let mut context = self.context.into_owned();
        context.push((key, value.into()));
        self.context = Cow::from(context);
        Box::new(self)
    }

    /// Overwrite context of the error.
    ///
    /// # Caution
    ///
    /// This should not be used in most cases, as it will overwrite any existing contexts.
    /// Rather use `attach_context` for multiple contexts.
    pub fn overwrite_context(self, value: impl Into<Vec<(&'static str, SmolStr)>>) -> Box<Self> {
        Box::new(Self {
            context: Cow::from(value.into()),
            ..self
        })
    }

    /// Attach the URL of the documentation for the error.
    pub fn attach_docs_url(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            docs_url: Some(value.into()),
            ..self
        })
    }

    /// Attach an error code.
    pub fn attach_error_code(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            error_code: Some(value.into()),
            ..self
        })
    }

    /// Attach the URL of the issue tracker for opening a new issue.
    pub fn attach_new_issue_url(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            new_issue_url: Some(value.into()),
            ..self
        })
    }

    /// Attach the URL of an already existing issue that is related to this error.
    pub fn attach_existing_issue_url(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            existing_issue_url: Some(value.into()),
            ..self
        })
    }

    /// Attach the severity of the error.
    pub fn attach_severity(self, value: impl Into<Severity>) -> Box<Self> {
        Box::new(Self {
            severity: Some(value.into()),
            ..self
        })
    }

    /// Attach the status of the error.
    pub fn attach_status(self, value: impl Into<Status>) -> Box<Self> {
        Box::new(Self {
            status: Some(value.into()),
            ..self
        })
    }

    /// Overwrite the backtrace of the error.
    ///
    /// This should not be used in most cases, as the backtrace is automatically captured.
    pub fn overwrite_backtrace(self, value: impl Into<Backtrace>) -> Box<Self> {
        Box::new(Self {
            backtrace: Some(value.into()),
            ..self
        })
    }
}

/// Severity of an error, ranging from informational to fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Informational
    Info,

    /// Warning
    Warning,

    /// Error
    Error,

    /// Fatal
    Fatal,
}

/// Status of an error, indicating whether it is permanent, temporary, or persistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Permanent, may not be retried
    Permanent,

    /// Temporary, may be retried
    Temporary,

    /// Persistent, may be retried, but may not succeed
    Persistent,
}

/// [`ErrorKind`] describes the errors that can happen while executing a high-level command.
///
/// This is a non-exhaustive enum, so additional variants may be added in future. It is
/// recommended to match against the wildcard `_` instead of listing all possible variants,
/// to avoid problems when new variants are added.
#[non_exhaustive]
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum ErrorKind {
    /// Append-only mode is enabled
    AppendOnly,
    /// Backend Error
    Backend,
    /// Command Error
    Command,
    /// Config Error
    Config,
    /// Crypto Error
    Cryptography,
    /// External Command Error
    ExternalCommand,
    /// Blob, Pack, Index or Tree Error
    // These are deep errors that are not expected to be handled by the user.
    Internal,
    /// Invalid Input Error
    InvalidInput,
    /// IO Error
    Io,
    /// Key Error
    Key,
    /// Parsing Error
    Parsing,
    /// Password Error
    Password,
    /// Repository Error
    Repository,
    /// Something is not supported
    Unsupported,
    /// Verification Error
    Verification,
    /// Virtual File System Error
    Vfs,
}
