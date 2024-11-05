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
//! ### Downgrading and Forwarding
//!
//! `RusticError`s should **not** be downgraded, instead we **upgrade** the function signature to contain a `RusticResult`.
//! For instance, if a function returns `Result<T, ArchiverErrorKind>` and we discover an error path that contains a `RusticError`,
//! we don't need to convert that into an `ArchiverErrorKind`, we should change the function signature, so it returns either a
//! `Result<T, RusticError> (==RusticResult<T>)` or nested results like `RusticResult<Result<T, ArchiverErrorKind>>`.
//! So even if the visibility of that function is `fn` or `pub(crate) fn` it should return a `RusticResult` containing a `RusticError`.
//!
//! If we `map_err` or `and_then` a `RusticError`, we don't want to create a new RusticError from it, but just attach some context
//! to it, e.g. `map_err(|e| e.attach_context("key", "value"))`, so we don't lose the original error. We can also change the error
//! kind with `map_err(|e| e.overwrite_kind(ErrorKind::NewKind))`. If we want to pre- or append to the guidance, we can use
//! `map_err(|e| e.append_guidance_line("new line"))` or `map_err(|e| e.prepend_guidance_line("new line"))`.
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

use derive_more::derive::Display;
use ecow::{EcoString, EcoVec};
use std::{
    backtrace::{Backtrace, BacktraceStatus},
    convert::Into,
    fmt::{self, Display},
};

pub(crate) mod constants {
    pub const DEFAULT_DOCS_URL: &str = "https://rustic.cli.rs/docs/errors/";
    pub const DEFAULT_ISSUE_URL: &str = "https://github.com/rustic-rs/rustic_core/issues/new";
}

/// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
pub type RusticResult<T, E = Box<RusticError>> = Result<T, E>;

/// Severity of an error, ranging from informational to fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum Status {
    /// Permanent, may not be retried
    Permanent,

    /// Temporary, may be retried
    Temporary,

    /// Persistent, may be retried, but may not succeed
    Persistent,
}

// NOTE:
//
// we use `an error related to {kind}` in the Display impl, so the variant display comments
// should be able to be used in a sentence.
//
/// [`ErrorKind`] describes the errors that can happen while executing a high-level command.
///
/// This is a non-exhaustive enum, so additional variants may be added in future. It is
/// recommended to match against the wildcard `_` instead of listing all possible variants,
/// to avoid problems when new variants are added.
#[non_exhaustive]
#[derive(thiserror::Error, Debug, displaydoc::Display, Default, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// append-only mode
    AppendOnly,
    /// the backend
    Backend,
    /// the configuration
    Configuration,
    /// cryptographic operations
    Cryptography,
    /// running an external command
    ExternalCommand,
    /// internal operations
    // Blob, Pack, Index, Tree Errors
    // Compression, Parsing, Multithreading etc.
    // These are deep errors that are not expected to be handled by the user.
    Internal,
    /// invalid user input
    InvalidInput,
    /// input/output operations
    InputOutput,
    /// a key
    Key,
    /// missing user input
    MissingInput,
    /// general operations
    #[default]
    Other,
    /// password handling
    Password,
    /// the repository
    Repository,
    /// unsupported operations
    Unsupported,
    /// verification
    Verification,
    /// the virtual filesystem
    Vfs,
}

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
/// Errors that can result from rustic.
pub struct RusticError {
    /// The kind of the error.
    kind: ErrorKind,

    /// The error message with guidance.
    guidance: EcoString,

    /// The URL of the documentation for the error.
    docs_url: Option<EcoString>,

    /// Error code.
    error_code: Option<EcoString>,

    /// Whether to ask the user to report the error.
    ask_report: bool,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_urls: EcoVec<EcoString>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<EcoString>,

    /// The context of the error.
    context: EcoVec<(EcoString, EcoString)>,

    /// Chain to the cause of the error.
    source: Option<Box<(dyn std::error::Error + Send + Sync)>>,

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
        writeln!(f, "Well, this is embarrassing.")?;
        writeln!(
            f,
            "\n`rustic_core` experienced an error related to `{}`.",
            self.kind
        )?;

        writeln!(f, "\nMessage:")?;
        if self.context.is_empty() {
            writeln!(f, "{}", self.guidance)?;
        } else {
            // If there is context, we want to iterate over it
            // use the key to replace the placeholder in the guidance.
            let mut guidance = self.guidance.to_string();
            self.context.iter().for_each(|(key, value)| {
                let pattern = "{".to_owned() + key + "}";
                guidance = guidance.replace(&pattern, value);
            });
            writeln!(f, "{guidance}")?;
        }

        if let Some(code) = &self.error_code {
            let default_docs_url = EcoString::from(constants::DEFAULT_DOCS_URL);
            let docs_url = self
                .docs_url
                .as_ref()
                .unwrap_or(&default_docs_url)
                .to_string();

            // If the docs_url doesn't end with a slash, add one.
            let docs_url = if docs_url.ends_with('/') {
                docs_url
            } else {
                docs_url + "/"
            };

            writeln!(f, "\nFor more information, see: {docs_url}{code}")?;
        }

        if !self.existing_issue_urls.is_empty() {
            writeln!(f, "\nRelated issues:")?;
            self.existing_issue_urls
                .iter()
                .try_for_each(|url| writeln!(f, "- {url}"))?;
        }

        if self.ask_report {
            let default_issue_url = EcoString::from(constants::DEFAULT_ISSUE_URL);
            let new_issue_url = self.new_issue_url.as_ref().unwrap_or(&default_issue_url);

            writeln!(
                f,
                "\nWe believe this is a bug, please report it by opening an issue at:"
            )?;
            writeln!(f, "{new_issue_url}")?;
            writeln!(
                f,
                "\nIf you can, please attach an anonymized debug log to the issue."
            )?;
            writeln!(f, "\nThank you for helping us improve rustic!")?;
        }

        writeln!(f, "\n\nSome additional details ...")?;

        if !self.context.is_empty() {
            writeln!(f, "\nContext:")?;
            self.context
                .iter()
                .try_for_each(|(key, value)| writeln!(f, "- {key}: {value}"))?;
        }

        if let Some(cause) = &self.source {
            writeln!(f, "\nCaused by:")?;
            writeln!(f, "{cause} : (source: {:?})", cause.source())?;
        }

        if let Some(severity) = &self.severity {
            writeln!(f, "\nSeverity: {severity}")?;
        }

        if let Some(status) = &self.status {
            writeln!(f, "\nStatus: {status}")?;
        }

        if let Some(backtrace) = &self.backtrace {
            writeln!(f, "\nBacktrace:")?;
            writeln!(f, "{backtrace}")?;

            if backtrace.status() == BacktraceStatus::Disabled {
                writeln!(
                    f,
                    "\nTo enable backtraces, set the RUST_BACKTRACE=\"1\" environment variable."
                )?;
            }
        }

        Ok(())
    }
}

// Accessors for anything we do want to expose publicly.
impl RusticError {
    /// Creates a new error with the given kind and guidance.
    pub fn new(kind: ErrorKind, guidance: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: guidance.into(),
            context: EcoVec::default(),
            source: None,
            error_code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_urls: EcoVec::default(),
            severity: None,
            status: None,
            ask_report: false,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        })
    }

    /// Creates a new error with the given kind and guidance.
    pub fn with_source(
        kind: ErrorKind,
        guidance: impl Into<EcoString>,
        source: impl Into<Box<(dyn std::error::Error + Send + Sync)>>,
    ) -> Box<Self> {
        Self::new(kind, guidance).attach_source(source)
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
        kind: ErrorKind,
        error: T,
    ) -> Box<Self> {
        Self::with_source(kind, error.to_string(), error)
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

    /// Ask the user to report the error.
    pub fn ask_report(self) -> Box<Self> {
        Box::new(Self {
            ask_report: true,
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
    pub fn overwrite_guidance(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            guidance: value.into(),
            ..self
        })
    }

    /// Append a newline to the guidance message.
    /// This is useful for adding additional information to the guidance.
    pub fn append_guidance_line(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            guidance: format!("{}\n{}", self.guidance, value.into()).into(),
            ..self
        })
    }

    /// Prepend a newline to the guidance message.
    /// This is useful for adding additional information to the guidance.
    pub fn prepend_guidance_line(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            guidance: format!("{}\n{}", value.into(), self.guidance).into(),
            ..self
        })
    }

    // IMPORTANT: This is manually implemented to allow for multiple contexts to be added.
    /// Attach context to the error.
    pub fn attach_context(
        mut self,
        key: impl Into<EcoString>,
        value: impl Into<EcoString>,
    ) -> Box<Self> {
        self.context.push((key.into(), value.into()));
        Box::new(self)
    }

    /// Overwrite context of the error.
    ///
    /// # Caution
    ///
    /// This should not be used in most cases, as it will overwrite any existing contexts.
    /// Rather use `attach_context` for multiple contexts.
    pub fn overwrite_context(self, value: impl Into<EcoVec<(EcoString, EcoString)>>) -> Box<Self> {
        Box::new(Self {
            context: EcoVec::from(value.into()),
            ..self
        })
    }

    /// Attach the URL of the documentation for the error.
    pub fn attach_docs_url(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            docs_url: Some(value.into()),
            ..self
        })
    }

    /// Attach an error code.
    pub fn attach_error_code(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            error_code: Some(value.into()),
            ..self
        })
    }

    /// Attach the URL of the issue tracker for opening a new issue.
    pub fn attach_new_issue_url(self, value: impl Into<EcoString>) -> Box<Self> {
        Box::new(Self {
            new_issue_url: Some(value.into()),
            ..self
        })
    }

    /// Attach the URL of an already existing issue that is related to this error.
    pub fn attach_existing_issue_url(mut self, value: impl Into<EcoString>) -> Box<Self> {
        self.existing_issue_urls.push(value.into());
        Box::new(self)
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
