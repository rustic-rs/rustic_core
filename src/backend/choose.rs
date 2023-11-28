use std::sync::Arc;

use crate::{
    backend::{local::LocalBackend, rclone::RcloneBackend, rest::RestBackend, WriteBackend},
    error::{BackendErrorKind, RusticResult},
};

/// Choose the suitable backend from a given url.
///
/// # Arguments
///
/// * `url` - The url to create the [`ChooseBackend`] from.
///
/// # Errors
///
/// * [`BackendErrorKind::BackendNotSupported`] - If the backend is not supported.
/// * [`LocalErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
/// * [`RestErrorKind::UrlParsingFailed`] - If the url could not be parsed.
/// * [`RestErrorKind::BuildingClientFailed`] - If the client could not be built.
///
/// [`BackendErrorKind::BackendNotSupported`]: crate::error::BackendErrorKind::BackendNotSupported
/// [`LocalErrorKind::DirectoryCreationFailed`]: crate::error::LocalErrorKind::DirectoryCreationFailed
/// [`RestErrorKind::UrlParsingFailed`]: crate::error::RestErrorKind::UrlParsingFailed
/// [`RestErrorKind::BuildingClientFailed`]: crate::error::RestErrorKind::BuildingClientFailed
pub fn choose_from_url(
    url: &str,
    options: impl IntoIterator<Item = (String, String)>,
) -> RusticResult<Arc<dyn WriteBackend>> {
    Ok(match url.split_once(':') {
        #[cfg(windows)]
        Some((drive, _)) if drive.len() == 1 => Arc::new(LocalBackend::new(url, options)?),
        Some(("rclone", path)) => Arc::new(RcloneBackend::new(path, options)?),
        Some(("rest", path)) => Arc::new(RestBackend::new(path, options)?),
        Some(("local", path)) => Arc::new(LocalBackend::new(path, options)?),
        Some((backend, _)) => {
            return Err(BackendErrorKind::BackendNotSupported(backend.to_owned()).into())
        }
        None => Arc::new(LocalBackend::new(url, options)?),
    })
}
