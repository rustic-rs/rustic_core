//! Error types and Result module.

// FIXME: Remove when 'displaydoc' has fixed/recommended further treatment upstream: https://github.com/yaahc/displaydoc/issues/48
#![allow(clippy::doc_markdown)]
// use std::error::Error as StdError;
// use std::fmt;

use derive_setters::Setters;
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
pub type RusticResult<T, E = RusticError> = Result<T, E>;

#[derive(thiserror::Error, Debug, Setters)]
#[setters(strip_option)]
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
    code: Option<SmolStr>,

    /// The URL of the issue tracker for opening a new issue.
    new_issue_url: Option<SmolStr>,

    /// The URL of an already existing issue that is related to this error.
    existing_issue_url: Option<SmolStr>,

    /// Severity of the error.
    severity: Option<Severity>,

    /// The status of the error.
    status: Option<Status>,

    /// Backtrace of the error.
    backtrace: Option<Backtrace>,
}

impl Display for RusticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "An error occurred in `rustic_core`: {}", self.kind)?;

        write!(f, "\nMessage: {}", self.guidance)?;

        if !self.context.is_empty() {
            write!(f, "\n\n Context:\n")?;
            write!(
                f,
                "{}",
                self.context
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
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

        if let Some(code) = &self.code {
            let default_docs_url = SmolStr::from(constants::DEFAULT_DOCS_URL);
            let docs_url = self.docs_url.as_ref().unwrap_or(&default_docs_url);

            write!(f, "\n\nFor more information, see: {docs_url}/{code}")?;
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
    pub fn new(kind: ErrorKind, guidance: impl Into<String>) -> Self {
        Self {
            kind,
            guidance: guidance.into().into(),
            context: Box::default(),
            source: None,
            code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
            status: None,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        }
    }

    /// Checks if the error has a specific error code.
    pub fn is_code(&self, code: &str) -> bool {
        self.code.as_ref().map_or(false, |c| c.as_str() == code)
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
    ) -> Self {
        Self {
            kind,
            guidance: error.to_string().into(),
            context: Box::default(),
            source: Some(Box::new(error)),
            code: None,
            docs_url: None,
            new_issue_url: None,
            existing_issue_url: None,
            severity: None,
            status: None,
            // `Backtrace::capture()` will check if backtrace has been enabled
            // internally. It's zero cost if backtrace is disabled.
            backtrace: Some(Backtrace::capture()),
        }
    }

    /// Adds a context to the error.
    #[must_use]
    pub fn add_context(mut self, key: &'static str, value: impl Into<String>) -> Self {
        let mut context = self.context.to_vec();
        context.push((key, value.into().into()));
        self.context = context.into_boxed_slice();
        self
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
    /// IO Error
    Io,
    /// Password Error
    Password,
    /// Repository Error
    Repository,
    /// Command Error
    Command,
    /// Config Error
    Config,
    /// Index Error
    Index,
    /// Key Error
    Key,
    /// Blob Error
    Blob,
    /// Crypto Error
    Cryptography,
    /// Compression Error
    Compression,
    /// Parsing Error
    Parsing,
    /// Conversion Error
    Conversion,
    /// Permission Error
    Permission,
    /// Polynomial Error
    Polynomial,
    /// Multithreading Error
    Multithreading,
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

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    static TEST_ERROR: LazyLock<Error> = LazyLock::new(|| RusticError {
        kind: ErrorKind::Io,
        guidance:
            "A file could not be read, make sure the file is existing and readable by the system."
                .to_string(),
        status: Some(Status::Permanent),
        severity: Some(Severity::Error),
        code: Some("E001".to_string().into()),
        context: vec![
            ("path", "/path/to/file".to_string()),
            ("called", "used s3 backend".to_string()),
        ],
        source: Some(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "networking error",
        ))),
        backtrace: Some(Backtrace::disabled()),
        docs_url: None,
        new_issue_url: None,
        existing_issue_url: None,
    });

    #[test]
    fn test_error_display() {
        todo!("Implement test_error_display");
    }

    #[test]
    fn test_error_debug() {
        todo!("Implement test_error_debug");
    }
}
