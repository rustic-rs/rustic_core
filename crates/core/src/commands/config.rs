//! `config` subcommand
use bytesize::ByteSize;
use derive_setters::Setters;

use crate::{
    backend::decrypt::{DecryptBackend, DecryptWriteBackend},
    crypto::CryptoKey,
    error::{ErrorKind, RusticError, RusticResult},
    repofile::ConfigFile,
    repository::{Open, Repository},
};

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
/// * If the version is not supported.
/// * If the version is lower than the current version.
/// * If compression is set for a v1 repo.
/// * If the compression level is not supported.
/// * If the size is too large.
/// * If the min pack size tolerance percent is wrong.
/// * If the max pack size tolerance percent is wrong.
/// * If the file could not be serialized to json.
///
/// # Returns
///
/// Whether the config was changed
pub(crate) fn apply_config<P, S: Open>(
    repo: &Repository<P, S>,
    opts: &ConfigOptions,
) -> RusticResult<bool> {
    if repo.config().append_only == Some(true) {
        return Err(RusticError::new(
            ErrorKind::AppendOnly,
            "Changing config is not allowed in append-only repositories. Please disable append-only mode first, if you know what you are doing. Aborting.",
        ));
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
/// * If the file could not be serialized to json.
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
    /// * If the version is not supported
    /// * If the version is lower than the current version
    /// * If compression is set for a v1 repo
    /// * If the compression level is not supported
    /// * If the size is too large
    /// * If the min packsize tolerate percent is wrong
    /// * If the max packsize tolerate percent is wrong
    pub fn apply(&self, config: &mut ConfigFile) -> RusticResult<()> {
        if let Some(version) = self.set_version {
            // only allow versions 1 and 2
            let range = 1..=2;

            if !range.contains(&version) {
                return Err(RusticError::new(
                    ErrorKind::Unsupported,
                    "Config version unsupported. Allowed versions are `{allowed_versions}`. You provided `{current_version}`. Please use a supported version. ",
                )
                .attach_context("current_version", version.to_string())
                .attach_context("allowed_versions", format!("{range:?}")));
            } else if version < config.version {
                return Err(RusticError::new(
                    ErrorKind::Unsupported,
                    "Downgrading config version is unsupported. You provided `{new_version}` which is smaller than `{current_version}`. Please use a version that is greater or equal to the current one.",
                )
                .attach_context("current_version", config.version.to_string())
                .attach_context("new_version", version.to_string()));
            }

            config.version = version;
        }

        if let Some(compression) = self.set_compression {
            if config.version == 1 && compression != 0 {
                return Err(RusticError::new(
                    ErrorKind::Unsupported,
                    "Compression `{compression}` unsupported for v1 repos.",
                )
                .attach_context("compression", compression.to_string()));
            }

            let range = zstd::compression_level_range();
            if !range.contains(&compression) {
                return Err(RusticError::new(
                    ErrorKind::Unsupported,
                    "Compression level `{compression}` is unsupported. Allowed levels are `{allowed_levels}`. Please use a supported level.",
                )
                .attach_context("compression", compression.to_string())
                .attach_context("allowed_levels", format!("{range:?}")));
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
                return Err(RusticError::new(
                    ErrorKind::InvalidInput,
                    "`min_packsize_tolerate_percent` must be <= 100. You provided `{percent}`.",
                )
                .attach_context("percent", percent.to_string()));
            }

            config.min_packsize_tolerate_percent = Some(percent);
        }

        if let Some(percent) = self.set_max_packsize_tolerate_percent {
            if percent < 100 && percent > 0 {
                return Err(RusticError::new(
                    ErrorKind::InvalidInput,
                    "`max_packsize_tolerate_percent` must be >= 100 or 0. You provided `{percent}`.",
                )
                .attach_context("percent", percent.to_string()));
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
        "Failed to convert ByteSize `{size}` to u64. Size is too large.",
        err,
    )
    .attach_context("size", size.to_string())
}
