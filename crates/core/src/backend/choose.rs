//! This module contains the trait [`BackendChoice`] and the function [`get_backend`] to choose a backend from a given url.

use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use crate::{
    backend::{url_to_type_and_path, WriteBackend},
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
        location: &str,
        options: Option<HashMap<String, String>>,
    ) -> Result<Arc<dyn WriteBackend>>;
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
    backend_type: Arc<dyn BackendChoice>,
    options: impl Into<Option<HashMap<String, String>>>,
) -> RusticResult<Arc<dyn WriteBackend>> {
    let (tpe, path) = url_to_type_and_path(url);
    backend_type
        .init(path, options.into())
        .map_err(|err| BackendAccessErrorKind::BackendLoadError(tpe.to_string(), err).into())
}

impl std::fmt::Debug for (dyn BackendChoice + 'static) {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: Implement Debug for BackendChoice
        write!(f, "BackendChoice")
    }
}
