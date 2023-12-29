pub mod choose;
pub mod error;
pub mod local;
pub mod opendal;
pub mod rclone;
pub mod rest;
pub mod util;

// rustic_backend Public API
pub use crate::{
    choose::{BackendOptions, SupportedBackend},
    local::LocalBackend,
    opendal::OpenDALBackend,
    rclone::RcloneBackend,
    rest::RestBackend,
};
