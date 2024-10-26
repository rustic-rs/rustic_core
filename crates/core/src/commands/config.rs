//! `config` subcommand
use std::ops::RangeInclusive;

use bytesize::ByteSize;
use derive_setters::Setters;

use crate::{
    backend::decrypt::{DecryptBackend, DecryptWriteBackend},
    crypto::CryptoKey,
    error::{ErrorKind, RusticError, RusticResult},
    repofile::ConfigFile,
    repository::{Open, Repository},
};

#[non_exhaustive]
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum ConfigCommandErrorKind {
    /// Not allowed on an append-only repository: `{0}`
    NotAllowedWithAppendOnly(String),
    /// compression level `{0}` is not supported for repo v1
    NoCompressionV1Repo(i32),
    /// version `{0}` is not supported. Allowed values: {1:?}
    VersionNotSupported(u32, RangeInclusive<u32>),
    /// compression level `{0}` is not supported. Allowed values: `{1:?}`
    CompressionLevelNotSupported(i32, RangeInclusive<i32>),
    /// cannot downgrade version from `{0}` to `{1}`
    CannotDowngrade(u32, u32),
    /// Size is too large: `{0}`
    SizeTooLarge(ByteSize),
    /// `min_packsize_tolerate_percent` must be <= 100
    MinPackSizeTolerateWrong,
    /// `max_packsize_tolerate_percent` must be >= 100 or 0"
    MaxPackSizeTolerateWrong,
}

pub(crate) type ConfigCommandResult<T> = Result<T, ConfigCommandErrorKind>;

/// Apply the [`ConfigOptions`] to a given [`ConfigFile`]
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to apply the config to
/// * `opts` - The options to apply
///
/// # Errors
///
/// * [`ConfigCommandErrorKind::VersionNotSupported`] - If the version is not supported
/// * [`ConfigCommandErrorKind::CannotDowngrade`] - If the version is lower than the current version
/// * [`ConfigCommandErrorKind::NoCompressionV1Repo`] - If compression is set for a v1 repo
/// * [`ConfigCommandErrorKind::CompressionLevelNotSupported`] - If the compression level is not supported
/// * [`ConfigCommandErrorKind::SizeTooLarge`] - If the size is too large
/// * [`ConfigCommandErrorKind::MinPackSizeTolerateWrong`] - If the min packsize tolerance percent is wrong
/// * [`ConfigCommandErrorKind::MaxPackSizeTolerateWrong`] - If the max packsize tolerance percent is wrong
/// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
///
/// # Returns
///
/// Whether the config was changed
///
/// [`ConfigCommandErrorKind::VersionNotSupported`]: ConfigCommandErrorKind::VersionNotSupported
/// [`ConfigCommandErrorKind::CannotDowngrade`]: ConfigCommandErrorKind::CannotDowngrade
/// [`ConfigCommandErrorKind::NoCompressionV1Repo`]: ConfigCommandErrorKind::NoCompressionV1Repo
/// [`ConfigCommandErrorKind::CompressionLevelNotSupported`]: ConfigCommandErrorKind::CompressionLevelNotSupported
/// [`ConfigCommandErrorKind::SizeTooLarge`]: ConfigCommandErrorKind::SizeTooLarge
/// [`ConfigCommandErrorKind::MinPackSizeTolerateWrong`]: ConfigCommandErrorKind::MinPackSizeTolerateWrong
/// [`ConfigCommandErrorKind::MaxPackSizeTolerateWrong`]: ConfigCommandErrorKind::MaxPackSizeTolerateWrong
/// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
pub(crate) fn apply_config<P, S: Open>(
    repo: &Repository<P, S>,
    opts: &ConfigOptions,
) -> RusticResult<bool> {
    if repo.config().append_only == Some(true) {
        return Err(ConfigCommandErrorKind::NotAllowedWithAppendOnly(
            "config change".to_string(),
        ))
        .map_err(|_err| todo!("Error transition"));
    }

    let mut new_config = repo.config().clone();
    opts.apply(&mut new_config)?;
    if &new_config == repo.config() {
        Ok(false)
    } else {
        save_config(repo, new_config, *repo.dbe().key())?;
        Ok(true)
    }
}

/// Save a [`ConfigFile`] to the repository
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to save the config to
/// * `new_config` - The config to save
/// * `key` - The key to encrypt the config with
///
/// # Errors
///
/// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
///
/// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
pub(crate) fn save_config<P, S>(
    repo: &Repository<P, S>,
    mut new_config: ConfigFile,
    key: impl CryptoKey,
) -> RusticResult<()> {
    new_config.is_hot = None;
    let dbe = DecryptBackend::new(repo.be.clone(), key);
    // for hot/cold backend, this only saves the config to the cold repo.
    _ = dbe.save_file_uncompressed(&new_config)?;

    if let Some(hot_be) = repo.be_hot.clone() {
        // save config to hot repo
        let dbe = DecryptBackend::new(hot_be.clone(), key);
        new_config.is_hot = Some(true);
        _ = dbe.save_file_uncompressed(&new_config)?;
    }

    Ok(())
}

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Debug, Clone, Copy, Default, Setters)]
#[setters(into)]
#[non_exhaustive]
/// Options for the `config` command, used to set repository-wide options
pub struct ConfigOptions {
    /// Set compression level. Allowed levels are 1 to 22 and -1 to -7, see <https://facebook.github.io/zstd/>.
    /// Note that 0 equals to no compression
    #[cfg_attr(feature = "clap", clap(long, value_name = "LEVEL"))]
    pub set_compression: Option<i32>,

    /// Set repository version. Allowed versions: 1,2
    #[cfg_attr(feature = "clap", clap(long, value_name = "VERSION"))]
    pub set_version: Option<u32>,

    /// Set append-only mode.
    /// Note that only append-only commands work once this is set. `forget`, `prune` or `config` won't work any longer.
    #[cfg_attr(feature = "clap", clap(long))]
    pub set_append_only: Option<bool>,

    /// Set default packsize for tree packs. rustic tries to always produce packs greater than this value.
    /// Note that for large repos, this value is grown by the grown factor.
    /// Defaults to `4 MiB` if not set.
    #[cfg_attr(feature = "clap", clap(long, value_name = "SIZE"))]
    pub set_treepack_size: Option<ByteSize>,

    /// Set upper limit for default packsize for tree packs.
    /// Note that packs actually can get a bit larger.
    /// If not set, pack sizes can grow up to approximately `4 GiB`.
    #[cfg_attr(feature = "clap", clap(long, value_name = "SIZE"))]
    pub set_treepack_size_limit: Option<ByteSize>,

    /// Set grow factor for tree packs. The default packsize grows by the square root of the total size of all
    /// tree packs multiplied with this factor. This means 32 kiB times this factor per square root of total
    /// treesize in GiB.
    /// Defaults to `32` (= 1MB per square root of total treesize in GiB) if not set.
    #[cfg_attr(feature = "clap", clap(long, value_name = "FACTOR"))]
    pub set_treepack_growfactor: Option<u32>,

    /// Set default packsize for data packs. rustic tries to always produce packs greater than this value.
    /// Note that for large repos, this value is grown by the grown factor.
    /// Defaults to `32 MiB` if not set.
    #[cfg_attr(feature = "clap", clap(long, value_name = "SIZE"))]
    pub set_datapack_size: Option<ByteSize>,

    /// Set grow factor for data packs. The default packsize grows by the square root of the total size of all
    /// data packs multiplied with this factor. This means 32 kiB times this factor per square root of total
    /// datasize in GiB.
    /// Defaults to `32` (= 1MB per square root of total datasize in GiB) if not set.
    #[cfg_attr(feature = "clap", clap(long, value_name = "FACTOR"))]
    pub set_datapack_growfactor: Option<u32>,

    /// Set upper limit for default packsize for tree packs.
    /// Note that packs actually can get a bit larger.
    /// If not set, pack sizes can grow up to approximately `4 GiB`.
    #[cfg_attr(feature = "clap", clap(long, value_name = "SIZE"))]
    pub set_datapack_size_limit: Option<ByteSize>,

    /// Set minimum tolerated packsize in percent of the targeted packsize.
    /// Defaults to `30` if not set.
    #[cfg_attr(feature = "clap", clap(long, value_name = "PERCENT"))]
    pub set_min_packsize_tolerate_percent: Option<u32>,

    /// Set maximum tolerated packsize in percent of the targeted packsize
    /// A value of `0` means packs larger than the targeted packsize are always
    /// tolerated. Default if not set: larger packfiles are always tolerated.
    #[cfg_attr(feature = "clap", clap(long, value_name = "PERCENT"))]
    pub set_max_packsize_tolerate_percent: Option<u32>,

    /// Do an extra verification by decompressing/decrypting all data before uploading to the repository.
    /// Default: true
    #[cfg_attr(feature = "clap", clap(long))]
    pub set_extra_verify: Option<bool>,
}

impl ConfigOptions {
    /// Apply the [`ConfigOptions`] to a given [`ConfigFile`]
    ///
    /// # Arguments
    ///
    /// * `config` - The config to apply the options to
    ///
    /// # Errors
    ///
    /// * [`ConfigCommandErrorKind::VersionNotSupported`] - If the version is not supported
    /// * [`ConfigCommandErrorKind::CannotDowngrade`] - If the version is lower than the current version
    /// * [`ConfigCommandErrorKind::NoCompressionV1Repo`] - If compression is set for a v1 repo
    /// * [`ConfigCommandErrorKind::CompressionLevelNotSupported`] - If the compression level is not supported
    /// * [`ConfigCommandErrorKind::SizeTooLarge`] - If the size is too large
    /// * [`ConfigCommandErrorKind::MinPackSizeTolerateWrong`] - If the min packsize tolerate percent is wrong
    /// * [`ConfigCommandErrorKind::MaxPackSizeTolerateWrong`] - If the max packsize tolerate percent is wrong
    ///
    /// [`ConfigCommandErrorKind::VersionNotSupported`]: ConfigCommandErrorKind::VersionNotSupported
    /// [`ConfigCommandErrorKind::CannotDowngrade`]: ConfigCommandErrorKind::CannotDowngrade
    /// [`ConfigCommandErrorKind::NoCompressionV1Repo`]: ConfigCommandErrorKind::NoCompressionV1Repo
    /// [`ConfigCommandErrorKind::CompressionLevelNotSupported`]: ConfigCommandErrorKind::CompressionLevelNotSupported
    /// [`ConfigCommandErrorKind::SizeTooLarge`]: ConfigCommandErrorKind::SizeTooLarge
    /// [`ConfigCommandErrorKind::MinPackSizeTolerateWrong`]: ConfigCommandErrorKind::MinPackSizeTolerateWrong
    /// [`ConfigCommandErrorKind::MaxPackSizeTolerateWrong`]: ConfigCommandErrorKind::MaxPackSizeTolerateWrong
    pub fn apply(&self, config: &mut ConfigFile) -> RusticResult<()> {
        if let Some(version) = self.set_version {
            let range = 1..=2;
            if !range.contains(&version) {
                return Err(ConfigCommandErrorKind::VersionNotSupported(version, range))
                    .map_err(|_err| todo!("Error transition"));
            } else if version < config.version {
                return Err(ConfigCommandErrorKind::CannotDowngrade(
                    config.version,
                    version,
                ))
                .map_err(|_err| todo!("Error transition"));
            }
            config.version = version;
        }

        if let Some(compression) = self.set_compression {
            if config.version == 1 && compression != 0 {
                return Err(ConfigCommandErrorKind::NoCompressionV1Repo(compression))
                    .map_err(|_err| todo!("Error transition"));
            }
            let range = zstd::compression_level_range();
            if !range.contains(&compression) {
                return Err(ConfigCommandErrorKind::CompressionLevelNotSupported(
                    compression,
                    range,
                ))
                .map_err(|_err| todo!("Error transition"));
            }
            config.compression = Some(compression);
        }

        if let Some(append_only) = self.set_append_only {
            config.append_only = Some(append_only);
        }

        if let Some(size) = self.set_treepack_size {
            config.treepack_size = Some(
                size.as_u64()
                    .try_into()
                    .map_err(|err| construct_size_too_large_error(err, size))?,
            );
        }
        if let Some(factor) = self.set_treepack_growfactor {
            config.treepack_growfactor = Some(factor);
        }
        if let Some(size) = self.set_treepack_size_limit {
            config.treepack_size_limit = Some(
                size.as_u64()
                    .try_into()
                    .map_err(|err| construct_size_too_large_error(err, size))?,
            );
        }

        if let Some(size) = self.set_datapack_size {
            config.datapack_size = Some(
                size.as_u64()
                    .try_into()
                    .map_err(|err| construct_size_too_large_error(err, size))?,
            );
        }
        if let Some(factor) = self.set_datapack_growfactor {
            config.datapack_growfactor = Some(factor);
        }
        if let Some(size) = self.set_datapack_size_limit {
            config.datapack_size_limit = Some(
                size.as_u64()
                    .try_into()
                    .map_err(|err| construct_size_too_large_error(err, size))?,
            );
        }

        if let Some(percent) = self.set_min_packsize_tolerate_percent {
            if percent > 100 {
                return Err(ConfigCommandErrorKind::MinPackSizeTolerateWrong)
                    .map_err(|_err| todo!("Error transition"));
            }
            config.min_packsize_tolerate_percent = Some(percent);
        }

        if let Some(percent) = self.set_max_packsize_tolerate_percent {
            if percent < 100 && percent > 0 {
                return Err(ConfigCommandErrorKind::MaxPackSizeTolerateWrong)
                    .map_err(|_err| todo!("Error transition"));
            }
            config.max_packsize_tolerate_percent = Some(percent);
        }

        config.extra_verify = self.set_extra_verify;

        Ok(())
    }
}

fn construct_size_too_large_error(
    err: std::num::TryFromIntError,
    size: ByteSize,
) -> Box<RusticError> {
    RusticError::with_source(
        ErrorKind::Internal,
        "Failed to convert ByteSize to u64. Size is too large.",
        err,
    )
    .attach_context("size", size.to_string())
}
