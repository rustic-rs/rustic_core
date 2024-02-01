/*!
A library for supporting various backends in rustic.

# Overview

This section gives a brief overview of the primary types in this crate:

<!-- OLD, KEPT AS TEMPLATE

The main type is the [`Repository`] type which describes a way to access a repository.
It can be in different states and allows - depending on the state - various high-level
actions to be performed on the repository like listing snapshots, backing up or restoring.

Besides this, various `*Option` types exist which allow to specify options for accessing a
[`Repository`] or for the methods used within a [`Repository`]. Those types usually offer
setter methods as well as implement [`serde::Serialize`] and [`serde::Deserialize`].

Other main types are typically result types obtained by [`Repository`] methods which sometimes
are also needed as input for other [`Repository`] method, like computing a [`PrunePlan`] and
performing it.

There are also lower level data types which represent the stored repository format or
help accessing/writing it. Those are collected in the [`repofile`] module. These types typically
implement [`serde::Serialize`] and [`serde::Deserialize`]. -->

# Example

```rust
<TODO> EXAMPLE!
```

# Crate features

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
