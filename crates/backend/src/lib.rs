/*!
A library for supporting various backends in rustic.

# Overview

This section gives a brief overview of the primary types in this crate:

`rustic_backend` is a support crate for `rustic_core` which provides a way to access a
repository using different backends.

The primary types in this crate are:

- `BackendOptions` - A struct for configuring options for a used backend.
- `SupportedBackend` - An enum for the supported backends.

The following backends are currently supported and can be enabled with features:

- `LocalBackend` - Backend for accessing a local filesystem.
- `OpenDALBackend` - Backend for accessing a `OpenDAL` filesystem.
- `RcloneBackend` - Backend for accessing a Rclone filesystem.
- `RestBackend` - Backend for accessing a REST API.

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
  enables the `conflate` dependency. This is needed for parsing commandline
  arguments and merging them into one (e.g. `config`). *This feature is disabled
  by default*.

### Backend-related features

- **opendal** - Enables support for the `opendal` backend. *This feature is
  enabled by default*.

- **rclone** - Enables support for the `rclone` backend. *This feature is
  enabled by default*.

- **rest** - Enables support for the `rest` backend. *This feature is enabled by
  default*.
*/

// formatting args are used for error messages
#![allow(clippy::literal_string_with_formatting_args)]

pub mod choose;
/// Local backend for Rustic.
pub mod local;
/// Utility functions for the backend.
pub mod util;

/// `OpenDAL` backend for Rustic.
#[cfg(feature = "opendal")]
pub mod opendal;

/// `Rclone` backend for Rustic.
#[cfg(feature = "rclone")]
pub mod rclone;

/// REST backend for Rustic.
#[cfg(feature = "rest")]
pub mod rest;

#[cfg(feature = "opendal")]
pub use crate::opendal::OpenDALBackend;

#[cfg(feature = "rclone")]
pub use crate::rclone::RcloneBackend;

#[cfg(feature = "rest")]
pub use crate::rest::RestBackend;

// rustic_backend Public API
pub use crate::{
    choose::{BackendOptions, SupportedBackend},
    local::LocalBackend,
};

// re-export for error handling
pub use rustic_core::{ErrorKind, RusticError, RusticResult, Severity, Status};
