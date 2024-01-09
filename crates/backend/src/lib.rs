pub mod choose;
pub mod error;
pub mod local;
#[cfg(feature = "opendal")]
pub mod opendal;
#[cfg(feature = "rclone")]
pub mod rclone;
#[cfg(feature = "rest")]
pub mod rest;
pub mod util;

// rustic_backend Public API
pub use crate::{
    choose::{BackendOptions, SupportedBackend},
    local::LocalBackend,
};

#[cfg(feature = "s3")]
pub use crate::opendal::s3::S3Backend;

#[cfg(feature = "opendal")]
pub use crate::opendal::OpenDALBackend;

#[cfg(feature = "rclone")]
pub use crate::rclone::RcloneBackend;

#[cfg(feature = "rest")]
pub use crate::rest::RestBackend;
