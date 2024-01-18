//! This module contains the trait [`BackendChoice`] and the function [`get_backend`] to choose a backend from a given url.
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
    #[cfg_attr(feature = "merge", merge(strategy = overwrite))]
    pub options: HashMap<String, String>,

    /// Other options for the hot repository
    #[cfg_attr(feature = "clap", clap(skip))]
    #[cfg_attr(feature = "merge", merge(strategy = overwrite))]
    pub options_hot: HashMap<String, String>,

    /// Other options for the cold repository
    #[cfg_attr(feature = "clap", clap(skip))]
    #[cfg_attr(feature = "merge", merge(strategy = overwrite))]
    pub options_cold: HashMap<String, String>,
}

/// Overwrite the left value with the right value
///
/// This is used for merging [`RepositoryOptions`] and [`ConfigOptions`]
///
/// # Arguments
///
/// * `left` - The left value
/// * `right` - The right value
#[cfg(feature = "merge")]
pub fn overwrite<T>(left: &mut T, right: T) {
    *left = right;
}

impl BackendOptions {
    pub fn to_backends(&self) -> Result<RepositoryBackends> {
        let mut options = self.options.clone();
        options.extend(self.options_cold.clone());
        let be = self
            .get_backend(self.repository.clone(), options)?
            .ok_or(anyhow!("Should be able to initialize main backend."))?;
        let mut options = self.options.clone();
        options.extend(self.options_hot.clone());
        let be_hot = self.get_backend(self.repo_hot.clone(), options)?;

        Ok(RepositoryBackends::new(be, be_hot))
    }

    fn get_backend(
        &self,
        repo_string: Option<String>,
        options: HashMap<String, String>,
    ) -> Result<Option<Arc<dyn WriteBackend>>> {
        repo_string
            .map(|string| {
                let (be_type, location) = location_to_type_and_path(&string)?;
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
#[derive(Debug, Clone, Copy, PartialEq, EnumString, Display)]
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
