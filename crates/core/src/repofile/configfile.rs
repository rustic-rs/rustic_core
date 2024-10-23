use std::num::ParseIntError;

use serde_derive::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    backend::FileType, blob::BlobType, define_new_id_struct, error::RusticResult, impl_repofile,
    repofile::RepoFile,
};

/// [`ConfigFileErrorKind`] describes the errors that can be returned for `ConfigFile`s
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum ConfigFileErrorKind {
    /// config version not supported: {version}, compression: {compression:?}
    ConfigVersionNotSupported {
        /// The version of the config
        version: u32,
        /// The compression level
        compression: Option<i32>,
    },
    /// Parsing failed for polynomial: {polynomial} : {source}
    ParsingFailedForPolynomial {
        polynomial: String,
        source: ParseIntError,
    },
}

pub(crate) type ConfigFileResult<T> = Result<T, ConfigFileErrorKind>;

pub(super) mod constants {

    pub(super) const KB: u32 = 1024;
    pub(super) const MB: u32 = 1024 * KB;

    /// Default Tree size
    pub(super) const DEFAULT_TREE_SIZE: u32 = 4 * MB;

    /// Default Data size
    pub(super) const DEFAULT_DATA_SIZE: u32 = 32 * MB;

    /// the default factor used for repo-size dependent pack size.
    /// 32 * sqrt(reposize in bytes) = 1 MB * sqrt(reposize in GB)
    pub(super) const DEFAULT_GROW_FACTOR: u32 = 32;

    /// The default maximum targeted pack size.
    pub(super) const DEFAULT_SIZE_LIMIT: u32 = u32::MAX;

    /// The default minimum percentage of targeted pack size.
    pub(super) const DEFAULT_MIN_PERCENTAGE: u32 = 30;
}

define_new_id_struct!(RepositoryId, "repository");
impl_repofile!(ConfigId, FileType::Config, ConfigFile);

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
/// The config file describes all repository-wide information.
///
/// It is usually saved in the repository as `config`
pub struct ConfigFile {
    /// Repository version. Currently 1 and 2 are supported
    pub version: u32,

    /// The [`Id`] identifying the repsitory
    pub id: RepositoryId,

    /// The chunker polynomial used to chunk data
    pub chunker_polynomial: String,

    /// Marker if this is a hot repository. If not set, this is no hot repository
    ///
    /// # Note
    ///
    /// When using hot/cold repositories, this is only set within the hot part of the repository.
    pub is_hot: Option<bool>,

    /// Marker if this is a append-only repository.
    ///
    /// # Note
    ///
    /// Commands which are not append-only won't run once this is set.
    pub append_only: Option<bool>,

    /// Compression level
    ///
    /// # Note
    ///
    /// `Some(0)` means no compression. If not set, use the default compression:
    /// * for repository version 1, use no compression (as not supported)
    /// * for repository version 2, use the zstd default compression
    pub compression: Option<i32>,

    /// Size of tree packs. This will be enhanced by the `treepack_growfactor` depending on the repository size
    ///
    /// If not set, defaults to 4 MiB
    pub treepack_size: Option<u32>,

    /// Grow factor to increase size of tree packs depending on the repository size
    ///
    /// If not set, defaults to `32`
    pub treepack_growfactor: Option<u32>,

    /// Maximum targeted tree pack size.
    pub treepack_size_limit: Option<u32>,

    /// Size of data packs. This will be enhanced by the `datapack_growfactor` depending on the repository size
    ///
    /// If not set, defaults to `32 MiB`
    pub datapack_size: Option<u32>,

    /// Grow factor to increase size of data packs depending on the repository size
    ///
    /// If not set, defaults to `32`
    pub datapack_growfactor: Option<u32>,

    /// maximum targeted data pack size.
    pub datapack_size_limit: Option<u32>,

    /// Tolerate pack sizes which are larger than given percentage of targeted pack size
    ///
    /// If not set, defaults to `30`
    pub min_packsize_tolerate_percent: Option<u32>,

    /// Tolerate pack sizes which are smaller than given percentage of targeted pack size
    ///
    /// If not set or set to `0` this is unlimited.
    pub max_packsize_tolerate_percent: Option<u32>,

    /// Do an extra verification by decompressing/decrypting all data before uploading to the repository
    pub extra_verify: Option<bool>,
}

impl ConfigFile {
    #[must_use]
    /// Creates a new `ConfigFile`.
    ///
    /// # Arguments
    ///
    /// * `version` - The version of the repository
    /// * `id` - The id of the repository
    /// * `poly` - The chunker polynomial
    pub fn new(version: u32, id: RepositoryId, poly: u64) -> Self {
        Self {
            version,
            id,
            chunker_polynomial: format!("{poly:x}"),
            ..Self::default()
        }
    }

    /// Get the chunker polynomial
    ///
    /// # Errors
    ///
    /// * [`ConfigFileErrorKind::ParsingFailedForPolynomial`] - If the polynomial could not be parsed
    ///
    /// [`ConfigFileErrorKind::ParsingFailedForPolynomial`]: ConfigFileErrorKind::ParsingFailedForPolynomial
    pub fn poly(&self) -> RusticResult<u64> {
        Ok(u64::from_str_radix(&self.chunker_polynomial, 16)
            .map_err(|err| ConfigFileErrorKind::ParsingFailedForPolynomial {
                polynomial: self.chunker_polynomial.clone(),
                source: err,
            })
            .map_err(|_err| todo!("Error transition"))?)
    }

    /// Get the compression level
    ///
    /// # Errors
    ///
    /// * [`ConfigFileErrorKind::ConfigVersionNotSupported`] - If the version is not supported
    ///
    /// [`ConfigFileErrorKind::ConfigVersionNotSupported`]: ConfigFileErrorKind::ConfigVersionNotSupported
    pub fn zstd(&self) -> RusticResult<Option<i32>> {
        match (self.version, self.compression) {
            (1, _) | (2, Some(0)) => Ok(None),
            (2, None) => Ok(Some(0)), // use default (=0) zstd compression
            (2, Some(c)) => Ok(Some(c)),
            _ => Err(ConfigFileErrorKind::ConfigVersionNotSupported {
                version: self.version,
                compression: self.compression,
            })
            .map_err(|_err| todo!("Error transition")),
        }
    }

    /// Get whether an extra verification (decompressing/decrypting data before writing to the repository) should be performed.
    #[must_use]
    pub fn extra_verify(&self) -> bool {
        self.extra_verify.unwrap_or(true) // default is to do the extra check
    }

    /// Get pack size parameter
    ///
    /// # Arguments
    ///
    /// * `blob` - The blob type to get the pack size parameters for
    ///
    /// # Returns
    ///
    /// A tuple containing the pack size, the grow factor and the size limit
    #[must_use]
    pub fn packsize(&self, blob: BlobType) -> (u32, u32, u32) {
        match blob {
            BlobType::Tree => (
                self.treepack_size.unwrap_or(constants::DEFAULT_TREE_SIZE),
                self.treepack_growfactor
                    .unwrap_or(constants::DEFAULT_GROW_FACTOR),
                self.treepack_size_limit
                    .unwrap_or(constants::DEFAULT_SIZE_LIMIT),
            ),
            BlobType::Data => (
                self.datapack_size.unwrap_or(constants::DEFAULT_DATA_SIZE),
                self.datapack_growfactor
                    .unwrap_or(constants::DEFAULT_GROW_FACTOR),
                self.datapack_size_limit
                    .unwrap_or(constants::DEFAULT_SIZE_LIMIT),
            ),
        }
    }

    /// Get pack size toleration limits
    ///
    /// # Returns
    ///
    ///
    #[must_use]
    pub fn packsize_ok_percents(&self) -> (u32, u32) {
        (
            self.min_packsize_tolerate_percent
                .unwrap_or(constants::DEFAULT_MIN_PERCENTAGE),
            match self.max_packsize_tolerate_percent {
                None | Some(0) => u32::MAX,
                Some(percent) => percent,
            },
        )
    }
}
