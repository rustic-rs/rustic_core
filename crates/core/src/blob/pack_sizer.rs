use super::BlobType;
use crate::repofile::ConfigFile;

use integer_sqrt::IntegerSquareRoot;

/// The pack sizer is responsible for computing the size of the pack file.
pub trait PackSizer {
    /// Computes the size of the pack file.
    #[must_use]
    fn pack_size(&self) -> u32;

    /// Evaluates whether the given size is not too small or too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    fn size_ok(&self, size: u32) -> bool {
        !self.is_too_small(size) && !self.is_too_large(size)
    }

    /// Evaluates whether the given size is too small
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    fn is_too_small(&self, _size: u32) -> bool {
        false
    }

    /// Evaluates whether the given size is too large
    ///
    /// # Arguments
    ///
    /// * `size` - The size to check
    #[must_use]
    fn is_too_large(&self, _size: u32) -> bool {
        false
    }

    /// Adds the given size to the current size.
    ///
    /// # Arguments
    ///
    /// * `added` - The size to add
    fn add_size(&mut self, _added: u32) {}
}

/// The default pack sizer computes packs depending on a default size, a grow factor amd a size limit.
#[derive(Debug, Clone, Copy)]
pub struct DefaultPackSizer {
    /// The default size of a pack file.
    default_size: u32,
    /// The grow factor of a pack file.
    grow_factor: u32,
    /// The size limit of a pack file.
    size_limit: u32,
    /// The current size of a pack file.
    current_size: u64,
    /// The minimum pack size tolerance in percent before a repack is triggered.
    min_packsize_tolerate_percent: u32,
    /// The maximum pack size tolerance in percent before a repack is triggered.
    max_packsize_tolerate_percent: u32,
}

impl DefaultPackSizer {
    /// Creates a new `DefaultPackSizer` from a config file.
    ///
    /// # Arguments
    ///
    /// * `config` - The config file.
    /// * `blob_type` - The blob type.
    /// * `current_size` - The current size of the pack file.
    ///
    /// # Returns
    ///
    /// A new `DefaultPackSizer`.
    #[must_use]
    pub fn from_config(config: &ConfigFile, blob_type: BlobType, current_size: u64) -> Self {
        let (default_size, grow_factor, size_limit) = config.packsize(blob_type);
        let (min_packsize_tolerate_percent, max_packsize_tolerate_percent) =
            config.packsize_ok_percents();
        Self {
            default_size,
            grow_factor,
            size_limit,
            current_size,
            min_packsize_tolerate_percent,
            max_packsize_tolerate_percent,
        }
    }
}

impl PackSizer for DefaultPackSizer {
    #[allow(clippy::cast_possible_truncation)]
    fn pack_size(&self) -> u32 {
        (self.current_size.integer_sqrt() as u32 * self.grow_factor + self.default_size)
            .min(self.size_limit)
    }

    fn is_too_small(&self, size: u32) -> bool {
        let target_size = self.pack_size();
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        u64::from(size) * 100
            < u64::from(target_size) * u64::from(self.min_packsize_tolerate_percent)
    }

    fn is_too_large(&self, size: u32) -> bool {
        let target_size = self.pack_size();
        // Note: we cast to u64 so that no overflow can occur in the multiplications
        u64::from(size) * 100
            > u64::from(target_size) * u64::from(self.max_packsize_tolerate_percent)
    }

    fn add_size(&mut self, added: u32) {
        self.current_size += u64::from(added);
    }
}

/// A pack sizer which uses a fixed pack size
#[derive(Debug, Clone, Copy)]
pub struct FixedPackSizer(pub u32);

impl PackSizer for FixedPackSizer {
    fn pack_size(&self) -> u32 {
        self.0
    }
    fn is_too_large(&self, size: u32) -> bool {
        size > self.0
    }
}
