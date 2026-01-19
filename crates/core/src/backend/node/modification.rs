use derive_setters::Setters;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{backend::node::ExtendedAttribute, repofile::Node};

/// Options how to treat times
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimeOption {
    /// Use the given time
    Yes,
    /// Use mtime
    Mtime,
    /// Use no time at all
    No,
}

impl TimeOption {
    /// Apply the `TimeOption`
    pub fn map_or_else(
        self,
        default: impl FnOnce() -> Option<Timestamp>,
        mtime: Option<Timestamp>,
    ) -> Option<Timestamp> {
        match self {
            Self::Yes => default(),
            Self::Mtime => mtime,
            Self::No => None,
        }
    }
}

/// Options how to set devid
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DevIdOption {
    /// Use the given devid
    Yes,
    /// Use the given devid if this is a hardlink, les not
    #[default]
    Hardlink,
    /// Use no devid at all
    No,
}

impl DevIdOption {
    /// Apply the `DevIdOption`
    pub fn map_or_else(self, dev_id: impl FnOnce() -> u64, hardlink: bool) -> u64 {
        match self {
            Self::Yes => dev_id(),
            Self::Hardlink if hardlink => dev_id(),
            _ => 0,
        }
    }
}

/// Options how block devices should be treated
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockdevOption {
    /// As special file, i.e. don't read the data
    #[default]
    Special,
    /// As normal file, i.e. read the data
    File,
}

/// Options how to set extended attributes
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum XattrOption {
    /// Use the given xattr
    #[default]
    Yes,
    /// Use no xattrs at all
    No,
}

impl XattrOption {
    /// Apply the `XattrOption`
    pub fn map_or_else(
        self,
        default: impl FnOnce() -> Vec<ExtendedAttribute>,
    ) -> Vec<ExtendedAttribute> {
        match self {
            Self::Yes => default(),
            Self::No => Vec::new(),
        }
    }
}

#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq, Eq, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
#[allow(clippy::struct_field_names)]
/// [`NodeModification`] describes how nodes will be modified
pub struct NodeModification {
    /// Set access time [default: mtime]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_atime: Option<TimeOption>,

    /// Set changed time [default: yes]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_ctime: Option<TimeOption>,

    /// Set device ID [default: hardlink]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_devid: Option<DevIdOption>,

    /// Set extended attributes [default: yes]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_xattrs: Option<XattrOption>,
}

impl NodeModification {
    #[must_use]
    /// Determines if no modification is in fact given
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    #[must_use]
    /// Modify the given node
    ///
    /// # Returns
    /// true if the node was changed else false
    pub fn modify_node(&self, node: &mut Node) -> bool {
        let mut meta = node.meta.clone();

        let mtime = meta.mtime;
        meta.atime = self
            .set_atime
            .unwrap_or(TimeOption::Mtime)
            .map_or_else(|| meta.atime, mtime);
        meta.ctime = self
            .set_ctime
            .unwrap_or(TimeOption::Yes)
            .map_or_else(|| meta.ctime, mtime);
        meta.device_id = self
            .set_devid
            .unwrap_or_default()
            .map_or_else(|| meta.device_id, meta.links > 1 && !node.is_dir());

        meta.extended_attributes = self
            .set_xattrs
            .unwrap_or_default()
            .map_or_else(|| meta.extended_attributes);

        let changed = node.meta != meta;
        node.meta = meta;
        changed
    }
}
