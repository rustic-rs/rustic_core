//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use ::std::convert::Into;
use smol_str::SmolStr;
use std::{
    backtrace::Backtrace,
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
    context: Box<[(&'static str, SmolStr)]>,

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
    pub fn new(kind: ErrorKind, guidance: impl Into<String>) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: guidance.into().into(),
            context: Box::default(),
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
            context: Box::default(),
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

    /// Expose the inner error kind.
    ///
    /// This is useful for matching on the error kind.
    pub fn into_inner(self) -> ErrorKind {
        self.kind
    }

    /// Checks if the error is due to an incorrect password
    pub fn is_incorrect_password(&self) -> bool {
        matches!(self.kind, ErrorKind::Password)
    }

    /// Creates a new error from a given error.
    pub fn from<T: std::error::Error + Display + Send + Sync + 'static>(
        error: T,
        kind: ErrorKind,
    ) -> Box<Self> {
        Box::new(Self {
            kind,
            guidance: error.to_string().into(),
            context: Box::default(),
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
    pub fn attach_kind(self, value: impl Into<ErrorKind>) -> Box<Self> {
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
    pub fn attach_guidance(self, value: impl Into<SmolStr>) -> Box<Self> {
        Box::new(Self {
            guidance: value.into(),
            ..self
        })
    }

    // IMPORTANT: This is manually implemented to allow for multiple contexts to be added.
    /// Attach context to the error.
    pub fn attach_context(mut self, key: &'static str, value: impl Into<SmolStr>) -> Box<Self> {
        let mut context = self.context.to_vec();
        context.push((key, value.into()));
        self.context = context.into_boxed_slice();
        Box::new(self)
    }

    /// Overwrite context of the error.
    ///
    /// # Caution
    ///
    /// This should not be used in most cases, as it will overwrite any existing contexts.
    /// Rather use `attach_context` for multiple contexts.
    pub fn overwrite_context(self, value: impl Into<Box<[(&'static str, SmolStr)]>>) -> Box<Self> {
        Box::new(Self {
            context: value.into(),
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

    /// Attach a backtrace of the error.
    ///
    /// This should not be used in most cases, as the backtrace is automatically captured.
    pub fn attach_backtrace_manually(self, value: impl Into<Backtrace>) -> Box<Self> {
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
    /// Backend Error
    Backend,
    /// Command Error
    Command,
    /// Compression Error
    Compression,
    /// Config Error
    Config,
    /// Crypto Error
    Cryptography,
    /// External Command Error
    ExternalCommand,
    /// Blob, Pack, Index or Tree Error
    // These are deep errors that are not expected to be handled by the user.
    Internal,
    /// IO Error
    Io,
    /// Key Error
    Key,
    /// Multithreading Error
    Multithreading,
    /// Parsing Error
    Parsing,
    /// Password Error
    Password,
    /// Permission Error
    Permission,
    /// Processing Error
    Processing,
    /// Repository Error
    Repository,
    /// Something is not supported
    Unsupported,
    /// Verification Error
    Verification,
    /// Virtual File System Error
    Vfs,
    // /// The repository password is incorrect. Please try again.
    // IncorrectRepositoryPassword,
    // /// No repository given. Please use the --repository option.
    // NoRepositoryGiven,
    // /// No password given. Please use one of the --password-* options.
    // NoPasswordGiven,
    // /// warm-up command must contain %id!
    // NoIDSpecified,
    // /// error opening password file `{0:?}`
    // OpeningPasswordFileFailed(std::io::Error),
    // /// No repository config file found. Is there a repo at `{0}`?
    // NoRepositoryConfigFound(String),
    // /// More than one repository config file at `{0}`. Aborting.
    // MoreThanOneRepositoryConfig(String),
    // /// keys from repo and repo-hot do not match for `{0}`. Aborting.
    // KeysDontMatchForRepositories(String),
    // /// repository is a hot repository!\nPlease use as --repo-hot in combination with the normal repo. Aborting.
    // HotRepositoryFlagMissing,
    // /// repo-hot is not a hot repository! Aborting.
    // IsNotHotRepository,
    // /// incorrect password!
    // IncorrectPassword,
    // /// error running the password command
    // PasswordCommandExecutionFailed,
    // /// error reading password from command
    // ReadingPasswordFromCommandFailed,
    // /// running command `{0}`:`{1}` was not successful: `{2}`
    // CommandExecutionFailed(String, String, std::io::Error),
    // /// running command {0}:{1} returned status: `{2}`
    // CommandErrorStatus(String, String, ExitStatus),
    // /// error listing the repo config file
    // ListingRepositoryConfigFileFailed,
    // /// error listing the repo keys
    // ListingRepositoryKeysFailed,
    // /// error listing the hot repo keys
    // ListingHotRepositoryKeysFailed,
    // /// error accessing config file
    // AccessToConfigFileFailed,
    // /// Thread pool build error: `{0:?}`
    // FromThreadPoolbilderError(rayon::ThreadPoolBuildError),
    // /// reading Password failed: `{0:?}`
    // ReadingPasswordFromReaderFailed(std::io::Error),
    // /// reading Password from prompt failed: `{0:?}`
    // ReadingPasswordFromPromptFailed(std::io::Error),
    // /// Config file already exists. Aborting.
    // ConfigFileExists,
    // /// did not find id `{0}` in index
    // IdNotFound(BlobId),
    // /// no suitable backend type found
    // NoBackendTypeGiven,
    // /// Hex decoding error: `{0:?}`
    // HexError(hex::FromHexError),
}

// TODO: Possible more general categories for errors for RusticErrorKind (WIP):
//
// - **JSON Parsing Errors**: e.g., `serde_json::Error`
// - **Version Errors**: e.g., `VersionNotSupported`, `CannotDowngrade`
// - **Compression Errors**: e.g., `NoCompressionV1Repo`, `CompressionLevelNotSupported`
// - **Size Errors**: e.g., `SizeTooLarge`
// - **File and Path Errors**: e.g., `ErrorCreating`, `ErrorCollecting`, `ErrorSettingLength`
// - **Thread Pool Errors**: e.g., `rayon::ThreadPoolBuildError`
// - **Conversion Errors**: e.g., `ConversionFromIntFailed`
// - **Permission Errors**: e.g., `NotAllowedWithAppendOnly`
// - **Parsing Errors**: e.g., `shell_words::ParseError`
// - **Cryptographic Errors**: e.g., `DataDecryptionFailed`, `DataEncryptionFailed`, `CryptoKeyTooShort`
// - **Polynomial Errors**: e.g., `NoSuitablePolynomialFound`
// - **File Handling Errors**: e.g., `TransposingOptionResultFailed`, `ConversionFromU64ToUsizeFailed`
// - **ID Processing Errors**: e.g., `HexError`
// - **Repository Errors**: general repository-related errors
// - **Backend Access Errors**: e.g., `BackendNotSupported`, `BackendLoadError`, `NoSuitableIdFound`, `IdNotUnique`
// - **Rclone Errors**: e.g., `NoOutputForRcloneVersion`, `NoStdOutForRclone`, `RCloneExitWithBadStatus`
// - **REST API Errors**: e.g., `NotSupportedForRetry`, `UrlParsingFailed`
