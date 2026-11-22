#[cfg(not(windows))]
pub mod nix_mapper;

use std::{ffi::OsStr, path::Path};

use derive_setters::Setters;
use ignore::DirEntry;
use jiff::Timestamp;
#[cfg(all(not(windows), not(target_os = "openbsd")))]
use log::debug;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::{IgnoreErrorKind, IgnoreResult, OpenFile};
use crate::backend::{
    ReadSourceEntry,
    node::{
        ExtendedAttribute, Metadata, Node, NodeType,
        modification::{DevIdOption, TimeOption, XattrOption},
    },
};

#[cfg(not(windows))]
use std::os::unix::fs::{FileTypeExt, MetadataExt};

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockdevOption {
    #[default]
    Special,
    File,
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
    pub fn map_entry(self, entry: DirEntry) -> IgnoreResult<ReadSourceEntry<OpenFile>> {
        let (entry, xattr_error) = self.map_entry_with_xattr_error(entry)?;
        if let Some(err) = xattr_error {
            warn!("Failed to read extended attributes: {err}");
        }
        Ok(entry)
    }

    pub(super) fn map_entry_with_xattr_error(
        self,
        entry: DirEntry,
    ) -> IgnoreResult<(ReadSourceEntry<OpenFile>, Option<IgnoreErrorKind>)> {
        let name = entry.file_name();
        let m = entry
            .metadata()
            .map_err(|err| IgnoreErrorKind::AcquiringMetadataFailed {
                name: name.to_string_lossy().to_string(),
                source: err,
            })?;

        let mtime = m.modified().ok().and_then(|t| Timestamp::try_from(t).ok());
        let atime = || m.accessed().ok().and_then(|t| Timestamp::try_from(t).ok());
        let atime = self
            .set_atime
            .unwrap_or(TimeOption::Mtime)
            .map_or_else(atime, mtime);
        let ctime = || Self::ctime(&m);
        let ctime = self
            .set_ctime
            .unwrap_or(TimeOption::Yes)
            .map_or_else(ctime, mtime);

        let (uid, user, gid, group) = Self::user_group(&m);
        let size = if m.is_dir() { 0 } else { m.len() };
        let device_id = self
            .set_devid
            .unwrap_or_default()
            .map_or_else(|| Self::device_id(&m), Self::hardlink(&m));
        let (extended_attributes, xattr_error) = match self.set_xattrs.unwrap_or_default() {
            XattrOption::Yes => Self::xattrs(entry.path()),
            XattrOption::No => (Vec::new(), None),
        };
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
        Ok((ReadSourceEntry { path, node, open }, xattr_error))
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
    fn ctime(m: &std::fs::Metadata) -> Option<Timestamp> {
        #[allow(clippy::cast_possible_truncation)]
        Timestamp::new(m.ctime(), m.ctime_nsec() as i32).ok()
    }

    fn device_id(m: &std::fs::Metadata) -> u64 {
        m.dev()
    }

    fn hardlink(m: &std::fs::Metadata) -> bool {
        m.nlink() > 1 && !m.is_dir()
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

    /// List [`ExtendedAttribute`]s for a [`Node`] located at `path`, preserving readable
    /// attributes even if another attribute fails to read.
    #[cfg(not(target_os = "openbsd"))]
    fn xattrs(path: &Path) -> (Vec<ExtendedAttribute>, Option<IgnoreErrorKind>) {
        let (attrs, error) =
            Self::collect_xattrs(path, xattr::list(path), |p, name| xattr::get(p, name));
        let error = error.map(|source| IgnoreErrorKind::ErrorXattr {
            path: path.to_path_buf(),
            source,
        });
        (attrs, error)
    }

    #[cfg(not(target_os = "openbsd"))]
    fn collect_xattrs(
        path: &Path,
        names: std::io::Result<impl IntoIterator<Item = std::ffi::OsString>>,
        mut get: impl FnMut(&Path, &OsStr) -> std::io::Result<Option<Vec<u8>>>,
    ) -> (Vec<ExtendedAttribute>, Option<std::io::Error>) {
        let names = match names {
            Ok(names) => names,
            Err(err) if err.kind() == std::io::ErrorKind::Unsupported => {
                debug!(
                    "extended attributes not supported for {}: {err}",
                    path.display()
                );
                return (Vec::new(), None);
            }
            Err(err) => return (Vec::new(), Some(err)),
        };

        let mut attrs = Vec::new();
        let mut first_error = None;
        for name in names {
            match get(path, &name) {
                Ok(Some(value)) => attrs.push(ExtendedAttribute {
                    name: name.to_string_lossy().to_string(),
                    value: Some(value),
                }),
                Ok(None) => {}
                Err(err) => {
                    // Count the affected entry once, and still read the remaining names.
                    _ = first_error.get_or_insert(err);
                }
            }
        }
        (attrs, first_error)
    }

    #[cfg(target_os = "openbsd")]
    fn xattrs(_path: &Path) -> (Vec<ExtendedAttribute>, Option<IgnoreErrorKind>) {
        (Vec::new(), None)
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
    fn ctime(m: &std::fs::Metadata) -> Option<Timestamp> {
        m.created().ok().and_then(|t| Timestamp::try_from(t).ok())
    }
    fn device_id(_m: &std::fs::Metadata) -> u64 {
        0
    }
    fn hardlink(m: &std::fs::Metadata) -> bool {
        false
    }
    fn user_group(
        _m: &std::fs::Metadata,
    ) -> (Option<u32>, Option<String>, Option<u32>, Option<String>) {
        (None, None, None, None)
    }

    fn nix_infos(_m: &std::fs::Metadata) -> (Option<u32>, u64, u64) {
        (None, 0, 0)
    }

    fn xattrs(_path: &Path) -> (Vec<ExtendedAttribute>, Option<IgnoreErrorKind>) {
        (Vec::new(), None)
    }

    fn to_node_other(self, name: &OsStr, _m: &std::fs::Metadata, meta: Metadata) -> Node {
        Node::new_node(name, NodeType::File, meta)
    }
}

#[cfg(all(test, not(windows), not(target_os = "openbsd")))]
mod tests {
    use std::{ffi::OsString, io};

    use super::*;

    #[test]
    fn failed_xattr_values_report_one_error_and_preserve_readable_values() {
        let (attrs, error) = LocalSourceSaveOptions::collect_xattrs(
            Path::new("file.txt"),
            Ok(["user.first", "user.unsupported", "user.denied", "user.last"].map(OsString::from)),
            |_, name| match name.to_str().unwrap() {
                "user.first" => Ok(Some(b"one".to_vec())),
                "user.unsupported" => Err(io::ErrorKind::Unsupported.into()),
                "user.denied" => Err(io::ErrorKind::PermissionDenied.into()),
                "user.last" => Ok(Some(b"two".to_vec())),
                _ => unreachable!(),
            },
        );

        assert_eq!(
            attrs,
            vec![
                ExtendedAttribute {
                    name: "user.first".to_string(),
                    value: Some(b"one".to_vec()),
                },
                ExtendedAttribute {
                    name: "user.last".to_string(),
                    value: Some(b"two".to_vec()),
                },
            ]
        );
        assert_eq!(error.unwrap().kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn unsupported_xattr_filesystem_does_not_report_error() {
        let (attrs, error) = LocalSourceSaveOptions::collect_xattrs(
            Path::new("sshfs_file.txt"),
            Err::<Vec<OsString>, _>(io::ErrorKind::Unsupported.into()),
            |_, _| unreachable!("listing an unsupported filesystem yields no names"),
        );

        assert_eq!(attrs, Vec::new());
        assert!(error.is_none());
    }

    #[test]
    fn vanished_xattr_between_list_and_get_is_skipped_without_error() {
        let (attrs, error) = LocalSourceSaveOptions::collect_xattrs(
            Path::new("race.txt"),
            Ok(["user.gone", "user.present"].map(OsString::from)),
            |_, name| {
                if name == "user.gone" {
                    Ok(None)
                } else {
                    Ok(Some(Vec::new()))
                }
            },
        );

        assert_eq!(
            attrs,
            vec![ExtendedAttribute {
                name: "user.present".to_string(),
                value: Some(Vec::new()),
            }]
        );
        assert!(error.is_none());
    }
}
