/*!
A library for supporting various backends in rustic.

# Overview

This section gives a brief overview of the primary types in this crate:

`rustic_backend` is a support crate for `rustic_core` which provides a way to access a
repository using different backends.

The primary types in this crate are:

- `BackendOptions` - A struct for configuring the backends to use.
- `SupportedBackend` - An enum for the supported backends.

The following backends are currently supported and can be enabled with features:

- `LocalBackend` - A backend for accessing the local filesystem.
- `OpenDALBackend` - A backend for accessing the OpenDAL filesystem.
- `RcloneBackend` - A backend for accessing the Rclone filesystem.
- `RestBackend` - A backend for accessing the REST API.
- `SftpBackend` - A backend for accessing the SFTP filesystem.
- `S3Backend` - A backend for accessing the S3 filesystem.

## Usage & Examples

Due to being a support crate for `rustic_core`, there are no examples here.
Please check the examples in the [`rustic_core`](https://crates.io/crates/rustic_core) crate.

## Crate features

This crate exposes a few features for controlling dependency usage:

- **cli** - Enables support for CLI features by enabling `merge` and `clap`
  features. *This feature is disabled by default*.

- **clap** - Enables a dependency on the `clap` crate and enables parsing from
  the commandline. *This feature is disabled by default*.

- **merge** - Enables support for merging multiple values into one, which
  enables the `merge` dependency. This is needed for parsing commandline
  arguments and merging them into one (e.g. `config`). *This feature is disabled
  by default*.

### Backend-related features

- **opendal** - Enables support for the `opendal` backend. *This feature is
  enabled by default*.
- **rclone** - Enables support for the `rclone` backend. *This feature is
  enabled by default*.

- **rest** - Enables support for the `rest` backend. *This feature is enabled by
  default*.

- **sftp** - Enables support for the `sftp` backend. Windows is not yet
  supported. *This feature is enabled by default*.

- **s3** - Enables support for the `s3` backend. *This feature is enabled by
  default*.
*/

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

#[cfg(all(unix, feature = "sftp"))]
pub use crate::opendal::sftp::SftpBackend;

#[cfg(feature = "s3")]
pub use crate::opendal::s3::S3Backend;

#[cfg(feature = "opendal")]
pub use crate::opendal::OpenDALBackend;

#[cfg(feature = "rclone")]
pub use crate::rclone::RcloneBackend;

#[cfg(feature = "rest")]
pub use crate::rest::RestBackend;
