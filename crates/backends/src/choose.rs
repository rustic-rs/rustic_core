use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use rustic_core::backend::{choose::BackendChoice, WriteBackend};

use crate::{
    error::BackendAccessErrorKind, local::LocalBackend, opendal::OpenDALBackend,
    rclone::RcloneBackend, rest::RestBackend,
};

/// The supported backend types.
///
/// Currently supported types are "local", "rclone", "rest", "opendal", "s3"
///
/// # Notes
///
/// If the url is a windows path, the type will be "local".
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum SupportedBackend {
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

impl TryFrom<&str> for SupportedBackend {
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

impl BackendChoice for SupportedBackend {
    fn init(
        &self,
        location: &str,
        options: Option<HashMap<String, String>>,
    ) -> Result<Arc<dyn WriteBackend>> {
        let options = options.unwrap_or_default();

        Ok(match self {
            Self::Local => Arc::new(LocalBackend::new(location, options)?),
            Self::Rclone => Arc::new(RcloneBackend::new(location, options)?),
            Self::Rest => Arc::new(RestBackend::new(location, options)?),
            Self::OpenDAL => Arc::new(OpenDALBackend::new(location, options)?),
            Self::S3 => Arc::new(OpenDALBackend::new_s3(location, options)?),
        })
    }
}

impl From<SupportedBackend> for Arc<dyn BackendChoice> {
    fn from(backend: SupportedBackend) -> Self {
        Arc::new(backend)
    }
}
