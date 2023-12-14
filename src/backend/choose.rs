use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use crate::{
    backend::{
        local::LocalBackend, opendal::OpenDALBackend, rclone::RcloneBackend, rest::RestBackend,
        WriteBackend,
    },
    error::BackendAccessErrorKind,
};

/// Choose the suitable backend from a given url.
///
/// # Arguments
///
/// * `tpe` - The backend type. Currently supported types are "local", "rclone", "rest", "opendal", "s3"
/// * `url` - The url to create the backend from.
/// * `options` - additional options for creating the backend
///
/// # Errors
///
/// * [`BackendAccessErrorKind::BackendNotSupported`] - If the backend is not supported.
///
/// [`BackendAccessErrorKind::BackendNotSupported`]: crate::error::BackendAccessErrorKind::BackendNotSupported
pub fn get_backend(
    tpe: &str,
    path: &str,
    options: HashMap<String, String>,
) -> Result<Arc<dyn WriteBackend>> {
    Ok(match tpe {
        "local" => Arc::new(LocalBackend::new(path, options)?),
        "rclone" => Arc::new(RcloneBackend::new(path, options)?),
        "rest" => Arc::new(RestBackend::new(path, options)?),
        "opendal" => Arc::new(OpenDALBackend::new(path, options)?),
        "s3" => Arc::new(OpenDALBackend::new_s3(path, options)?),
        backend => {
            return Err(BackendAccessErrorKind::BackendNotSupported(backend.to_owned()).into())
        }
    })
}

pub fn url_to_type_and_path(url: &str) -> (&str, &str) {
    match url.split_once(':') {
        #[cfg(windows)]
        Some((drive, _)) if drive.len() == 1 => ("local", url),
        Some((tpe, path)) => (tpe, path),
        None => ("local", url),
    }
}
