//! This module contains the trait [`BackendChoice`] and the function [`get_backend`] to choose a backend from a given url.
use anyhow::Result;
use derive_setters::Setters;
use serde_with::serde_as;
use std::{collections::HashMap, sync::Arc};
use strum_macros::{Display, EnumString};

#[allow(unused_imports)]
use rustic_core::overwrite;
use rustic_core::{backend::WriteBackend, RepositoryBackends};

use crate::{
    error::BackendAccessErrorKind,
    local::LocalBackend,
    opendal::OpenDALBackend,
    rclone::RcloneBackend,
    rest::RestBackend,
    util::{url_to_type_and_path, BackendUrl},
};

/// Options for a backend.
#[serde_as]
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

impl BackendOptions {
    pub fn to_backends(&self) -> Result<RepositoryBackends> {
        let be = self
            .get_backend(self.repository.clone())?
            .expect("Should be able to initialize main backend.");
        let be_hot = self.get_backend(self.repo_hot.clone())?;

        Ok(RepositoryBackends::new(be, be_hot))
    }

    fn get_backend(&self, repo_string: Option<String>) -> Result<Option<Arc<dyn WriteBackend>>> {
        repo_string
            .map(|string| {
                let (be_type, location) = url_to_type_and_path(&string)?;

                let mut options = self.options.clone();
                options.extend(self.options_cold.clone());

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
        location: BackendUrl,
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

    /// A rclone backend
    #[strum(serialize = "rclone", to_string = "rclone Backend")]
    Rclone,

    /// A REST backend
    #[strum(serialize = "rest", to_string = "REST Backend")]
    Rest,

    /// An openDAL backend (general)
    #[strum(serialize = "opendal", to_string = "openDAL Backend")]
    OpenDAL,

    /// An openDAL S3 backend
    #[strum(serialize = "s3", to_string = "S3 Backend")]
    S3,
}

impl BackendChoice for SupportedBackend {
    fn to_backend(
        &self,
        location: BackendUrl,
        options: Option<HashMap<String, String>>,
    ) -> Result<Arc<dyn WriteBackend>> {
        let options = options.unwrap_or_default();

        Ok(match self {
            Self::Local => Arc::new(LocalBackend::new(&location, options)?),
            Self::Rclone => Arc::new(RcloneBackend::new(&location, options)?),
            Self::Rest => Arc::new(RestBackend::new(&location, options)?),
            Self::OpenDAL => Arc::new(OpenDALBackend::new(&location, options)?),
            Self::S3 => Arc::new(OpenDALBackend::new_s3(&location, options)?),
        })
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local", SupportedBackend::Local)]
    #[case("rclone", SupportedBackend::Rclone)]
    #[case("rest", SupportedBackend::Rest)]
    #[case("opendal", SupportedBackend::OpenDAL)]
    #[case("s3", SupportedBackend::S3)]
    fn test_try_from_is_ok(#[case] input: &str, #[case] expected: SupportedBackend) {
        assert_eq!(SupportedBackend::try_from(input).unwrap(), expected);
    }

    #[test]
    fn test_try_from_unknown_is_err() {
        assert!(SupportedBackend::try_from("unknown").is_err());
    }
}
