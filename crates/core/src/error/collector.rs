use derive_more::From;

use crate::{error::RusticErrorKind, RusticError};
use log::{error, info, warn};

/// A rustic issue result
///
/// rustic issue results are used to return a result along with possible issues.
pub(crate) type RusticIssueResult<T> = Result<(T, Option<Vec<RusticIssue>>), Vec<RusticError>>;

/// A rustic issue
///
/// An issue is a message that can be logged to the user.
#[derive(Debug)]
pub(crate) enum RusticIssue {
    /// An error issue, indicating that something went wrong irrecoverably
    Error(RusticError),

    /// A warning issue, indicating that something might be wrong
    Warning(RusticWarning),

    /// An info issue, indicating additional information
    Info(RusticInfo),
}

impl RusticIssue {
    pub(crate) fn new_error(error: RusticErrorKind) -> Self {
        Self::Error(error.into())
    }

    pub(crate) fn new_warning(message: &str) -> Self {
        Self::Warning(message.into())
    }

    pub(crate) fn new_info(message: &str) -> Self {
        Self::Info(message.into())
    }

    pub(crate) fn log(&self) {
        match self {
            Self::Error(error) => error!("{}", error),
            Self::Warning(warning) => warn!("{}", warning.0),
            Self::Info(info) => info!("{}", info.0),
        }
    }
}

/// A rustic warning message
///
/// Warning messages are used to indicate that something might be wrong.
#[derive(Debug, Clone, From)]
pub(crate) struct RusticWarning(String);

impl RusticWarning {
    pub(crate) fn new(message: &str) -> Self {
        Self(message.to_owned())
    }
}

impl From<&str> for RusticWarning {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// A rustic info message
///
/// Info messages are used to provide additional information to the user.
#[derive(Debug, Clone, From)]
pub(crate) struct RusticInfo(String);

impl RusticInfo {
    pub(crate) fn new(message: &str) -> Self {
        Self(message.to_owned())
    }
}

impl From<&str> for RusticInfo {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RusticIssueCollector {
    /// The errors collected
    errors: Option<Vec<RusticError>>,

    /// The warnings collected
    warnings: Option<Vec<RusticWarning>>,

    /// The info collected
    info: Option<Vec<RusticInfo>>,

    /// Whether to log items directly during addition
    log: bool,
}

impl RusticIssueCollector {
    pub(crate) fn new(log: bool) -> Self {
        Self {
            errors: None,
            warnings: None,
            info: None,
            log,
        }
    }

    pub(crate) fn add(&mut self, issue: RusticIssue) {
        match issue {
            RusticIssue::Error(error) => self.add_error(error.0),
            RusticIssue::Warning(warning) => self.add_warning(&warning.0),
            RusticIssue::Info(info) => self.add_info(&info.0),
        }
    }

    pub(crate) fn add_error(&mut self, error: RusticErrorKind) {
        if self.log {
            error!("{error}");
        }

        if let Some(errors) = &mut self.errors {
            errors.push(error.into());
        } else {
            self.errors = Some(vec![error.into()]);
        }
    }

    pub(crate) fn add_warning(&mut self, message: &str) {
        if self.log {
            warn!("{message}");
        }

        if let Some(warnings) = &mut self.warnings {
            warnings.push(message.to_owned().into());
        } else {
            self.warnings = Some(vec![message.to_owned().into()]);
        }
    }

    pub(crate) fn add_info(&mut self, message: &str) {
        if self.log {
            warn!("{message}");
        }

        if let Some(info) = &mut self.info {
            info.push(message.to_owned().into());
        } else {
            self.info = Some(vec![message.to_owned().into()]);
        }
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.errors.is_some()
    }

    pub(crate) fn has_warnings(&self) -> bool {
        self.warnings.is_some()
    }

    pub(crate) fn has_info(&self) -> bool {
        self.info.is_some()
    }

    pub(crate) fn get_errors(&self) -> Option<Vec<&RusticError>> {
        self.errors.as_ref().map(|errors| errors.iter().collect())
    }

    pub(crate) fn get_warnings(&self) -> Option<Vec<RusticWarning>> {
        self.warnings.clone()
    }

    pub(crate) fn get_info(&self) -> Option<Vec<RusticInfo>> {
        self.info.clone()
    }

    pub(crate) fn log_all(&self) {
        self.log_all_errors();
        self.log_all_warnings();
        self.log_all_info();
    }

    pub(crate) fn log_all_errors(&self) {
        if let Some(errors) = &self.errors {
            for error in errors {
                error!("{}", error);
            }
        }
    }

    pub(crate) fn log_all_warnings(&self) {
        if let Some(warnings) = &self.warnings {
            for warning in warnings {
                warn!("{}", warning.0);
            }
        }
    }

    pub(crate) fn log_all_info(&self) {
        if let Some(info) = &self.info {
            for info in info {
                info!("{}", info.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RusticErrorKind;

    #[test]
    fn test_add_issue() {
        let mut collector = RusticIssueCollector::default();

        let issue = RusticIssue::new_error(RusticErrorKind::StdIo(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));

        collector.add(issue);
        assert!(collector.has_errors());
        assert!(!collector.has_warnings());
        assert!(!collector.has_info());
    }

    #[test]
    fn test_add_error() {
        let mut collector = RusticIssueCollector::default();
        collector.add_error(RusticErrorKind::StdIo(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));

        assert!(collector.has_errors());
        assert!(!collector.has_warnings());
        assert!(!collector.has_info());
    }

    #[test]
    fn test_add_warning() {
        let mut collector = RusticIssueCollector::default();
        collector.add_warning("test");
        assert!(!collector.has_errors());
        assert!(collector.has_warnings());
        assert!(!collector.has_info());
    }

    #[test]
    fn test_add_info() {
        let mut collector = RusticIssueCollector::default();
        collector.add_info("test");
        assert!(!collector.has_errors());
        assert!(!collector.has_warnings());
        assert!(collector.has_info());
    }

    #[test]
    fn test_get_errors() {
        let mut collector = RusticIssueCollector::default();
        collector.add_error(RusticErrorKind::StdIo(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));
        assert_eq!(collector.get_errors().unwrap().len(), 1);
    }

    #[test]
    fn test_get_warnings() {
        let mut collector = RusticIssueCollector::default();
        collector.add_warning("test");
        assert_eq!(collector.get_warnings().unwrap().len(), 1);
    }

    #[test]
    fn test_get_info() {
        let mut collector = RusticIssueCollector::default();
        collector.add_info("test");
        assert_eq!(collector.get_info().unwrap().len(), 1);
    }
}
