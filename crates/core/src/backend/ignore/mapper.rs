#[cfg(not(windows))]
pub mod nix_mapper;

use std::{ffi::OsStr, path::Path};

use derive_setters::Setters;
use ignore::DirEntry;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::{IgnoreErrorKind, IgnoreResult, OpenFile};
use crate::backend::{
    ReadSourceEntry,
    node::{ExtendedAttribute, Metadata, Node, NodeType},
};

#[cfg(not(windows))]
use {
    log::warn,
    std::os::unix::fs::{FileTypeExt, MetadataExt},
};

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimeOption {
    Yes,
    Mtime,
    No,
}

impl TimeOption {
    fn map(self, default: Option<Timestamp>, mtime: Option<Timestamp>) -> Option<Timestamp> {
        match self {
            Self::Yes => default,
            Self::Mtime => mtime,
            Self::No => None,
        }
    }
}

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DevIdOption {
    Yes,
    #[default]
    Hardlink,
    No,
}

impl DevIdOption {
    #[cfg(windows)]
    fn map(self, _m: &std::fs::Metadata) -> u64 {
        0
    }

    #[cfg(not(windows))]
    fn map(self, m: &std::fs::Metadata) -> u64 {
        match self {
            Self::Yes => m.dev(),
            Self::Hardlink if m.nlink() > 1 && !m.is_dir() => m.dev(),
            _ => 0,
        }
    }
}

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockdevOption {
    #[default]
    Special,
    File,
}

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum XattrOption {
    #[default]
    Yes,
    No,
}

impl XattrOption {
    #[cfg(any(windows, target_os = "openbsd"))]
    fn map(&self, _path: &Path) -> Vec<ExtendedAttribute> {
        Vec::new()
    }

    /// List [`ExtendedAttribute`] for a [`Node`] located at `path`
    ///
    /// # Argument
    ///
    /// * `path` to the [`Node`] for which to list attributes
    ///
    /// # Errors
    ///
    /// * If Xattr couldn't be listed or couldn't be read
    #[cfg(not(any(windows, target_os = "openbsd")))]
    fn map(self, path: &Path) -> Vec<ExtendedAttribute> {
        let list = |path: &Path| {
            xattr::list(path)
                .map_err(|err| IgnoreErrorKind::ErrorXattr {
                    path: path.to_path_buf(),
                    source: err,
                })?
                .map(|name| {
                    Ok(ExtendedAttribute {
                        name: name.to_string_lossy().to_string(),
                        value: xattr::get(path, name).map_err(|err| {
                            IgnoreErrorKind::ErrorXattr {
                                path: path.to_path_buf(),
                                source: err,
                            }
                        })?,
                    })
                })
                .collect::<IgnoreResult<Vec<ExtendedAttribute>>>()
        };

        match self {
            Self::Yes => list(path)
                .inspect_err(|err| {
                    warn!("ignoring error: {err}");
                })
                .unwrap_or_default(),
            Self::No => Vec::new(),
        }
    }
}

#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(serde::Deserialize, serde::Serialize, Default, Clone, Copy, Debug, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
/// [`LocalSourceSaveOptions`] describes how entries from a local source will be saved in the repository.
pub struct LocalSourceSaveOptions {
    /// Set access time [default: yes]
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

    /// How block devices should be stored [default: special]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_blockdev: Option<BlockdevOption>,

    /// Set extended attributes [default: yes]
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_xattrs: Option<XattrOption>,
}

impl LocalSourceSaveOptions {
    /// Maps a [`DirEntry`] to a [`ReadSourceEntry`].
    ///
    /// # Arguments
    ///
    /// * `entry` - The [`DirEntry`] to map.
    /// * `options` - options for saving entries
    ///
    /// # Errors
    ///
    /// * If metadata could not be read.
    /// * If the xattr of the entry could not be read.
    pub fn map_entry(self, entry: DirEntry) -> IgnoreResult<ReadSourceEntry<OpenFile>> {
        let name = entry.file_name();
        let m = entry
            .metadata()
            .map_err(|err| IgnoreErrorKind::AcquiringMetadataFailed {
                name: name.to_string_lossy().to_string(),
                source: err,
            })?;

        let mtime = m.modified().ok().and_then(|t| Timestamp::try_from(t).ok());
        let atime = m.accessed().ok().and_then(|t| Timestamp::try_from(t).ok());
        let atime = self
            .set_atime
            .unwrap_or(TimeOption::Mtime)
            .map(atime, mtime);
        let ctime = Self::default_ctime(&m);
        let ctime = self.set_ctime.unwrap_or(TimeOption::Yes).map(ctime, mtime);

        let (uid, user, gid, group) = Self::user_group(&m);
        let size = if m.is_dir() { 0 } else { m.len() };
        let device_id = self.set_devid.unwrap_or_default().map(&m);
        let extended_attributes = self.set_xattrs.unwrap_or_default().map(entry.path());
        let (mode, inode, links) = Self::nix_infos(&m);

        let meta = Metadata {
            mode,
            mtime,
            atime,
            ctime,
            uid,
            gid,
            user,
            group,
            inode,
            device_id,
            size,
            links,
            extended_attributes,
        };

        let node = self.to_node(&entry, &m, meta)?;
        let path = entry.into_path();
        let open = Some(OpenFile(path.clone()));
        Ok(ReadSourceEntry { path, node, open })
    }

    fn to_node(
        self,
        entry: &DirEntry,
        m: &std::fs::Metadata,
        meta: Metadata,
    ) -> IgnoreResult<Node> {
        let name = entry.file_name();
        let node = if m.is_dir() {
            Node::new_node(name, NodeType::Dir, meta)
        } else if m.is_symlink() {
            let path = entry.path();
            let target = std::fs::read_link(path).map_err(|err| IgnoreErrorKind::ErrorLink {
                path: path.to_path_buf(),
                source: err,
            })?;
            let node_type = NodeType::from_link(&target);
            Node::new_node(name, node_type, meta)
        } else {
            self.to_node_other(name, m, meta)
        };
        Ok(node)
    }
}

#[cfg(not(windows))]
impl LocalSourceSaveOptions {
    fn default_ctime(m: &std::fs::Metadata) -> Option<Timestamp> {
        #[allow(clippy::cast_possible_truncation)]
        Timestamp::new(m.ctime(), m.ctime_nsec() as i32).ok()
    }

    fn user_group(
        m: &std::fs::Metadata,
    ) -> (Option<u32>, Option<String>, Option<u32>, Option<String>) {
        let uid = m.uid();
        let gid = m.gid();
        let user = nix_mapper::get_user_by_uid(uid);
        let group = nix_mapper::get_group_by_gid(gid);
        (Some(uid), user, Some(gid), group)
    }

    fn nix_infos(m: &std::fs::Metadata) -> (Option<u32>, u64, u64) {
        let mode = nix_mapper::map_mode_to_go(m.mode());
        let inode = m.ino();
        let links = if m.is_dir() { 0 } else { m.nlink() };
        (Some(mode), inode, links)
    }

    fn to_node_other(self, name: &OsStr, m: &std::fs::Metadata, meta: Metadata) -> Node {
        let filetype = m.file_type();
        if filetype.is_block_device() {
            if matches!(self.set_blockdev.unwrap_or_default(), BlockdevOption::File) {
                Node::new_node(name, NodeType::File, meta)
            } else {
                let node_type = NodeType::Dev { device: m.rdev() };
                Node::new_node(name, node_type, meta)
            }
        } else if filetype.is_char_device() {
            let node_type = NodeType::Chardev { device: m.rdev() };
            Node::new_node(name, node_type, meta)
        } else if filetype.is_fifo() {
            Node::new_node(name, NodeType::Fifo, meta)
        } else if filetype.is_socket() {
            Node::new_node(name, NodeType::Socket, meta)
        } else {
            Node::new_node(name, NodeType::File, meta)
        }
    }
}

#[cfg(windows)]
impl LocalSourceSaveOptions {
    fn default_ctime(m: &std::fs::Metadata) -> Option<Timestamp> {
        m.created().ok().and_then(|t| Timestamp::try_from(t).ok())
    }
    fn user_group(
        _m: &std::fs::Metadata,
    ) -> (Option<u32>, Option<String>, Option<u32>, Option<String>) {
        (None, None, None, None)
    }

    fn nix_infos(_m: &std::fs::Metadata) -> (Option<u32>, u64, u64) {
        (None, 0, 0)
    }

    fn to_node_other(self, name: &OsStr, _m: &std::fs::Metadata, meta: Metadata) -> Node {
        Node::new_node(name, NodeType::File, meta)
    }
}
