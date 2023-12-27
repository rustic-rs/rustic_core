use anyhow::Result;
use std::{collections::HashMap, path::Path, sync::Arc};

use crate::{
    backend::{
        local::LocalBackend, opendal::OpenDALBackend, rclone::RcloneBackend, rest::RestBackend,
        WriteBackend,
    },
    error::BackendAccessErrorKind,
    RusticResult,
};

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
    fn init(
        &self,
        path: impl AsRef<Path>,
        options: HashMap<String, String>,
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
#[derive(Debug, Clone, Copy)]
pub enum BackendType {
    /// The local backend
    Local,

    /// A rclone backend
    Rclone,

    /// A REST backend
    Rest,

    /// A general openDAL backend
    OpenDAL,

    /// The s3 backend (backed by openDAL)
    S3,
}

impl TryFrom<&str> for BackendType {
    type Error = BackendAccessErrorKind;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "local" => Ok(Self::Local),
            "rclone" => Ok(Self::Rclone),
            "rest" => Ok(Self::Rest),
            "opendal" => Ok(Self::OpenDAL),
            "s3" => Ok(Self::S3),
            backend => Err(BackendAccessErrorKind::BackendNotSupported(
                backend.to_owned(),
            )),
        }
    }
}

impl BackendChoice for BackendType {
    fn init(
        &self,
        path: impl AsRef<Path>,
        options: HashMap<String, String>,
    ) -> Result<Arc<dyn WriteBackend>> {
        Ok(match self {
            Self::Local => Arc::new(LocalBackend::new(path, options)?),
            Self::Rclone => Arc::new(RcloneBackend::new(path, options)?),
            Self::Rest => Arc::new(RestBackend::new(path, options)?),
            Self::OpenDAL => Arc::new(OpenDALBackend::new(path, options)?),
            Self::S3 => Arc::new(OpenDALBackend::new_s3(path, options)?),
        })
    }
}

/// Choose the suitable backend from a given url.
///
/// # Arguments
///
/// * `url` - The url to create the backend from.
/// * `options` - additional options for creating the backend
///
/// # Errors
///
/// * [`BackendAccessErrorKind::BackendNotSupported`] - If the backend is not supported.
///
/// [`BackendAccessErrorKind::BackendNotSupported`]: crate::error::BackendAccessErrorKind::BackendNotSupported
pub fn get_backend(
    url: &str,
    options: HashMap<String, String>,
) -> RusticResult<Arc<dyn WriteBackend>> {
    let (tpe, path) = url_to_type_and_path(url);
    BackendType::try_from(tpe)?
        .init(path, options)
        .map_err(|err| BackendAccessErrorKind::BackendLoadError(tpe.to_string(), err).into())
}

/// Splits the given url into the backend type and the path.
///
/// # Arguments
///
/// * `url` - The url to split.
///
/// # Returns
///
/// A tuple with the backend type and the path.
///
/// # Notes
///
/// If the url is a windows path, the type will be "local".
pub fn url_to_type_and_path(url: &str) -> (&str, &str) {
    match url.split_once(':') {
        #[cfg(windows)]
        Some((drive, _)) if drive.len() == 1 => ("local", url),
        Some((tpe, path)) => (tpe, path),
        None => ("local", url),
    }
}
