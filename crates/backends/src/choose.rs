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
    fn init(&self, path: &str, options: HashMap<String, String>) -> Result<Arc<dyn WriteBackend>> {
        Ok(match self {
            Self::Local => Arc::new(LocalBackend::new(path, options)?),
            Self::Rclone => Arc::new(RcloneBackend::new(path, options)?),
            Self::Rest => Arc::new(RestBackend::new(path, options)?),
            Self::OpenDAL => Arc::new(OpenDALBackend::new(path, options)?),
            Self::S3 => Arc::new(OpenDALBackend::new_s3(path, options)?),
        })
    }
}
