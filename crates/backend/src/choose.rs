//! This module contains [`BackendOptions`] and helpers to choose a backend from a given url.
use anyhow::{anyhow, Result};
use derive_setters::Setters;
use std::{collections::HashMap, sync::Arc};
use strum_macros::{Display, EnumString};

#[allow(unused_imports)]
use rustic_core::{RepositoryBackends, WriteBackend};

use crate::{
    error::BackendAccessErrorKind,
    local::LocalBackend,
    util::{location_to_type_and_path, BackendLocation},
};

#[cfg(feature = "s3")]
use crate::opendal::s3::S3Backend;

#[cfg(all(unix, feature = "sftp"))]
use crate::opendal::sftp::SftpBackend;

#[cfg(feature = "opendal")]
use crate::opendal::OpenDALBackend;

#[cfg(feature = "rclone")]
use crate::rclone::RcloneBackend;

#[cfg(feature = "rest")]
use crate::rest::RestBackend;

/// Options for a backend.
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(merge::Merge))]
#[derive(Clone, Default, Debug, serde::Deserialize, serde::Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into, strip_option)]
pub struct BackendOptions {
    /// Repository to use
    #[cfg_attr(
        feature = "clap",
        clap(short, long, global = true, alias = "repo", env = "RUSTIC_REPOSITORY")
    )]
    pub repository: Option<String>,

    /// Repository to use as hot storage
    #[cfg_attr(
        feature = "clap",
        clap(long, global = true, alias = "repository_hot", env = "RUSTIC_REPO_HOT")
    )]
    pub repo_hot: Option<String>,

    /// Other options for this repository (hot and cold part)
    #[cfg_attr(feature = "clap", clap(skip))]
    #[cfg_attr(feature = "merge", merge(strategy = extend))]
    pub options: HashMap<String, String>,

    /// Other options for the hot repository
    #[cfg_attr(feature = "clap", clap(skip))]
    #[cfg_attr(feature = "merge", merge(strategy = extend))]
    pub options_hot: HashMap<String, String>,

    /// Other options for the cold repository
    #[cfg_attr(feature = "clap", clap(skip))]
    #[cfg_attr(feature = "merge", merge(strategy = extend))]
    pub options_cold: HashMap<String, String>,
}

/// Overwrite the left value with the right value
///
/// This is used for merging config values, e.g. [`BackendOptions`].
///
/// # Arguments
///
/// * `left` - The left value
/// * `right` - The right value
#[cfg(feature = "merge")]
pub fn extend<A, T: Extend<A> + IntoIterator<Item = A>>(left: &mut T, right: T) {
    left.extend(right);
}

impl BackendOptions {
    /// Convert the options to backends.
    ///
    /// # Errors
    ///
    /// If the repository is not given, an error is returned.
    ///
    /// # Returns
    ///
    /// The backends for the repository.
    pub fn to_backends(&self) -> Result<RepositoryBackends> {
        let mut options = self.options.clone();
        options.extend(self.options_cold.clone());
        let be = self
            .get_backend(self.repository.as_ref(), options)?
            .ok_or_else(|| anyhow!("No repository given."))?;
        let mut options = self.options.clone();
        options.extend(self.options_hot.clone());
        let be_hot = self.get_backend(self.repo_hot.as_ref(), options)?;

        Ok(RepositoryBackends::new(be, be_hot))
    }

    /// Get the backend for the given repository.
    ///
    /// # Arguments
    ///
    /// * `repo_string` - The repository string to use.
    /// * `options` - Additional options for the backend.
    ///
    /// # Errors
    ///
    /// If the backend cannot be loaded, an error is returned.
    ///
    /// # Returns
    ///
    /// The backend for the given repository.
    // Allow unused_self, as we want to access this method
    #[allow(clippy::unused_self)]
    fn get_backend(
        &self,
        repo_string: Option<&String>,
        options: HashMap<String, String>,
    ) -> Result<Option<Arc<dyn WriteBackend>>> {
        repo_string
            .map(|string| {
                let (be_type, location) = location_to_type_and_path(string)?;
                be_type.to_backend(location, options.into()).map_err(|err| {
                    BackendAccessErrorKind::BackendLoadError(be_type.to_string(), err).into()
                })
            })
            .transpose()
    }
}

/// Trait which can be implemented to choose a backend from a backend type, a backend path and options given as `HashMap`.
pub trait BackendChoice {
    /// Init backend from a path and options.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to create that points to the backend.
    /// * `options` - additional options for creating the backend
    ///
    /// # Errors
    ///
    /// * [`BackendAccessErrorKind::BackendNotSupported`] - If the backend is not supported.
    ///
    /// [`BackendAccessErrorKind::BackendNotSupported`]: crate::error::BackendAccessErrorKind::BackendNotSupported
    fn to_backend(
        &self,
        location: BackendLocation,
        options: Option<HashMap<String, String>>,
    ) -> Result<Arc<dyn WriteBackend>>;
}

/// The supported backend types.
///
/// Currently supported types are "local", "rclone", "rest", "opendal", "s3"
///
/// # Notes
///
/// If the url is a windows path, the type will be "local".
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, Display)]
pub enum SupportedBackend {
    /// A local backend
    #[strum(serialize = "local", to_string = "Local Backend")]
    Local,

    #[cfg(feature = "rclone")]
    /// A rclone backend
    #[strum(serialize = "rclone", to_string = "rclone Backend")]
    Rclone,

    #[cfg(feature = "rest")]
    /// A REST backend
    #[strum(serialize = "rest", to_string = "REST Backend")]
    Rest,

    #[cfg(feature = "opendal")]
    /// An openDAL backend (general)
    #[strum(serialize = "opendal", to_string = "openDAL Backend")]
    OpenDAL,

    #[cfg(feature = "s3")]
    /// An openDAL S3 backend
    #[strum(serialize = "s3", to_string = "S3 Backend")]
    S3,

    #[cfg(all(unix, feature = "sftp"))]
    /// An openDAL sftp backend
    #[strum(serialize = "sftp", to_string = "sftp Backend")]
    Sftp,
}

impl BackendChoice for SupportedBackend {
    fn to_backend(
        &self,
        location: BackendLocation,
        options: Option<HashMap<String, String>>,
    ) -> Result<Arc<dyn WriteBackend>> {
        let options = options.unwrap_or_default();

        Ok(match self {
            Self::Local => Arc::new(LocalBackend::new(location, options)?),
            #[cfg(feature = "rclone")]
            Self::Rclone => Arc::new(RcloneBackend::new(location, options)?),
            #[cfg(feature = "rest")]
            Self::Rest => Arc::new(RestBackend::new(location, options)?),
            #[cfg(feature = "opendal")]
            Self::OpenDAL => Arc::new(OpenDALBackend::new(location, options)?),
            #[cfg(feature = "s3")]
            Self::S3 => Arc::new(S3Backend::new(location, options)?),
            #[cfg(all(unix, feature = "sftp"))]
            Self::Sftp => Arc::new(SftpBackend::new(location, options)?),
        })
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local", SupportedBackend::Local)]
    #[cfg(feature = "rclone")]
    #[case("rclone", SupportedBackend::Rclone)]
    #[cfg(feature = "rest")]
    #[case("rest", SupportedBackend::Rest)]
    #[cfg(feature = "opendal")]
    #[case("opendal", SupportedBackend::OpenDAL)]
    #[cfg(feature = "s3")]
    #[case("s3", SupportedBackend::S3)]
    fn test_try_from_is_ok(#[case] input: &str, #[case] expected: SupportedBackend) {
        assert_eq!(SupportedBackend::try_from(input).unwrap(), expected);
    }

    #[test]
    fn test_try_from_unknown_is_err() {
        assert!(SupportedBackend::try_from("unknown").is_err());
    }
}
