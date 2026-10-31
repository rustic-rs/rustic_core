#[cfg(not(windows))]
use std::os::unix::fs::{PermissionsExt, symlink};

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    num::TryFromIntError,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use filetime::{FileTime, set_symlink_file_times};
#[cfg(not(windows))]
use log::warn;
#[cfg(not(windows))]
use nix::errno::Errno;
#[cfg(not(windows))]
use nix::sys::stat::{Mode, SFlag, mknod};
#[cfg(not(windows))]
use nix::{
    fcntl::{AT_FDCWD, AtFlags},
    unistd::{Gid, Uid, fchownat},
};
use same_file::is_same_file;

#[cfg(not(windows))]
use crate::backend::ignore::mapper::nix_mapper::map_mode_from_go;
#[cfg(not(windows))]
use crate::backend::node::NodeType;
use crate::{
    backend::node::{ExtendedAttribute, Metadata, Node},
    error::{ErrorKind, RusticError, RusticResult},
};

#[cfg(not(windows))]
mod helpers {
    // Helper function to cache mapping user name -> uid
    #![allow(clippy::needless_pass_by_value)]

    use cached::macros::cached;
    use log::warn;
    use nix::unistd::{Gid, Group, Uid, User};

    #[cached]
    pub(super) fn uid_from_name(name: String) -> Option<Uid> {
        User::from_name(&name)
            .inspect_err(|err| warn!("Cannot determine UID from name {name}: {err}. Using UID 0."))
            .unwrap_or_default()
            .map(|u| u.uid)
    }

    // Helper function to cache mapping group name -> gid
    #[cached]
    pub(super) fn gid_from_name(name: String) -> Option<Gid> {
        Group::from_name(&name)
            .inspect_err(|err| warn!("Cannot determine GID from name {name}: {err}. Using UID 0."))
            .unwrap_or_default()
            .map(|g| g.gid)
    }
}

/// [`LocalDestinationErrorKind`] describes the errors that can be returned by an action on the filesystem in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum LocalDestinationErrorKind {
    /// directory creation failed: `{0:?}`
    DirectoryCreationFailed(std::io::Error),
    /// item path `{0:?}` contains invalid components or escapes destination
    ItemPathEscapesDestination(PathBuf),
    /// file `{0:?}` should have a parent
    FileDoesNotHaveParent(PathBuf),
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "openbsd",
        all(target_os = "android", target_pointer_width = "32")
    ))]
    /// `DeviceID` could not be converted to other type `{target}` of device `{device}`: `{source}`
    DeviceIdConversionFailed {
        target: String,
        device: u64,
        source: TryFromIntError,
    },
    /// Length conversion failed for `{target}` of length `{length}`: `{source}`
    LengthConversionFailed {
        target: String,
        length: u64,
        source: TryFromIntError,
    },
    /// `{0}`
    #[error(transparent)]
    #[cfg(not(windows))]
    FromErrnoError(Errno),
    /// listing xattrs on `{path:?}`: `{source:?}`
    #[cfg(not(any(windows, target_os = "openbsd")))]
    ListingXattrsFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// setting xattr `{name}` on `{filename:?}` with `{source:?}`
    #[cfg(not(any(windows, target_os = "openbsd")))]
    SettingXattrFailed {
        name: String,
        filename: PathBuf,
        source: std::io::Error,
    },
    /// getting xattr `{name}` on `{filename:?}` with `{source:?}`
    #[cfg(not(any(windows, target_os = "openbsd")))]
    GettingXattrFailed {
        name: String,
        filename: PathBuf,
        source: std::io::Error,
    },
    /// removing directories failed: `{0:?}`
    DirectoryRemovalFailed(std::io::Error),
    /// removing file failed: `{0:?}`
    FileRemovalFailed(std::io::Error),
    /// setting time metadata failed: `{0:?}`
    SettingTimeMetadataFailed(std::io::Error),
    /// opening file failed: `{0:?}`
    OpeningFileFailed(std::io::Error),
    /// setting file length failed: `{0:?}`
    SettingFileLengthFailed(std::io::Error),
    /// can't jump to position in file: `{0:?}`
    CouldNotSeekToPositionInFile(std::io::Error),
    /// couldn't write to buffer: `{0:?}`
    CouldNotWriteToBuffer(std::io::Error),
    /// reading exact length of file contents failed: `{0:?}`
    ReadingExactLengthOfFileFailed(std::io::Error),
    /// setting file permissions failed: `{0:?}`
    #[cfg(not(windows))]
    SettingFilePermissionsFailed(std::io::Error),
    /// failed to symlink target `{linktarget:?}` from `{filename:?}` with `{source:?}`
    #[cfg(not(windows))]
    SymlinkingFailed {
        linktarget: PathBuf,
        filename: PathBuf,
        source: std::io::Error,
    },
    /// failed to create hardlink from `{source_path:?}` to `{filename:?}` with `{source:?}`
    HardLinkingFailed {
        source_path: PathBuf,
        filename: PathBuf,
        source: std::io::Error,
    },
}

pub(crate) type LocalDestinationResult<T> = Result<T, LocalDestinationErrorKind>;

#[derive(Clone, Debug)]
/// Local destination, used when restoring.
pub struct LocalDestination {
    /// The base path of the destination.
    path: PathBuf,
    /// Whether we expect a single file as destination.
    is_file: bool,
}

impl LocalDestination {
    /// Create a new [`LocalDestination`]
    ///
    /// # Arguments
    ///
    /// * `path` - The base path of the destination
    /// * `create` - If `create` is true, create the base path if it doesn't exist.
    /// * `expect_file` - Whether we expect a single file as destination.
    ///
    /// # Errors
    ///
    /// * If the directory could not be created.
    // TODO: We should use `impl Into<Path/PathBuf>` here. we even use it in the body!
    pub fn new(path: &str, create: bool, expect_file: bool) -> RusticResult<Self> {
        let is_dir = path.ends_with('/');
        let path: PathBuf = path.into();
        let is_file = path.is_file() || (!path.is_dir() && !is_dir && expect_file);

        // FIXME: Refactor logic to avoid duplication
        if create {
            if is_file {
                if let Some(path) = path.parent() {
                    fs::create_dir_all(path).map_err(|err| {
                        RusticError::with_source(
                            ErrorKind::InputOutput,
                            "The directory `{path}` could not be created.",
                            err,
                        )
                        .attach_context("path", path.display().to_string())
                    })?;
                }
            } else {
                fs::create_dir_all(&path).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InputOutput,
                        "The directory `{path}` could not be created.",
                        err,
                    )
                    .attach_context("path", path.display().to_string())
                })?;
            }
        }

        Ok(Self { path, is_file })
    }

    /// Path to the given item (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to get the path for
    ///
    /// # Returns
    ///
    /// The path to the item.
    ///
    /// # Notes
    ///
    /// * If the destination is a file, this will return the base path.
    /// * If the destination is a directory, this will return the base path joined with the item.
    pub(crate) fn path(&self, item: impl AsRef<Path>) -> LocalDestinationResult<PathBuf> {
        let item = item.as_ref();
        if self.is_file {
            return Ok(self.path.clone());
        }

        let mut res = self.path.clone();
        for comp in item.components() {
            match comp {
                std::path::Component::Prefix(_)
                | std::path::Component::RootDir
                | std::path::Component::ParentDir => {
                    return Err(LocalDestinationErrorKind::ItemPathEscapesDestination(
                        item.to_path_buf(),
                    ));
                }
                std::path::Component::CurDir => {}
                std::path::Component::Normal(c) => {
                    res.push(c);
                }
            }
        }
        Ok(res)
    }

    /// Remove the given directory (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `dirname` - The directory to remove
    ///
    /// # Errors
    ///
    /// * If the directory could not be removed.
    ///
    /// # Notes
    ///
    /// This will remove the directory recursively.
    #[allow(clippy::unused_self)]
    pub(crate) fn remove_dir(&self, dirname: impl AsRef<Path>) -> LocalDestinationResult<()> {
        fs::remove_dir_all(dirname).map_err(LocalDestinationErrorKind::DirectoryRemovalFailed)
    }

    /// Remove the given file (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `filename` - The file to remove
    ///
    /// # Errors
    ///
    /// * If the file could not be removed.
    ///
    /// # Notes
    ///
    /// This will remove the file.
    ///
    /// * If the file is a symlink, the symlink will be removed, not the file it points to.
    /// * If the file is a directory or device, this will fail.
    #[allow(clippy::unused_self)]
    pub(crate) fn remove_file(&self, filename: impl AsRef<Path>) -> LocalDestinationResult<()> {
        fs::remove_file(filename).map_err(LocalDestinationErrorKind::FileRemovalFailed)
    }

    /// Create the given directory (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The directory to create
    ///
    /// # Errors
    ///
    /// * If the directory could not be created.
    ///
    /// # Notes
    ///
    /// This will create the directory structure recursively.
    pub(crate) fn create_dir(&self, item: impl AsRef<Path>) -> LocalDestinationResult<()> {
        let dirname = self.path(item)?;
        fs::create_dir_all(dirname).map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;
        Ok(())
    }

    /// Set changed and modified times for `item` (relative to the base path) utilizing the file metadata
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the times for
    /// * `meta` - The metadata to get the times from
    ///
    /// # Errors
    ///
    /// * If the times could not be set
    pub(crate) fn set_times(
        &self,
        item: impl AsRef<Path>,
        meta: &Metadata,
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;
        if let Some(mtime) = meta.mtime {
            let atime = meta.atime.unwrap_or(mtime);
            set_symlink_file_times(
                filename,
                FileTime::from_system_time(atime.into()),
                FileTime::from_system_time(mtime.into()),
            )
            .map_err(LocalDestinationErrorKind::SettingTimeMetadataFailed)?;
        }

        Ok(())
    }

    #[cfg(windows)]
    // TODO: Windows support
    /// Set user/group for `item` (relative to the base path) utilizing the file metadata
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the user/group for
    /// * `meta` - The metadata to get the user/group from
    ///
    /// # Errors
    ///
    /// * If the user/group could not be set.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn set_user_group(
        &self,
        _item: impl AsRef<Path>,
        _meta: &Metadata,
    ) -> LocalDestinationResult<()> {
        // https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights
        // https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Security/struct.SECURITY_ATTRIBUTES.html
        // https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Storage/FileSystem/struct.CREATEFILE2_EXTENDED_PARAMETERS.html#structfield.lpSecurityAttributes
        Ok(())
    }

    #[cfg(not(windows))]
    /// Set user/group for `item` (relative to the base path) utilizing the file metadata
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the user/group for
    /// * `meta` - The metadata to get the user/group from
    ///
    /// # Errors
    ///
    /// * If the user/group could not be set.
    #[allow(clippy::similar_names)]
    pub(crate) fn set_user_group(
        &self,
        item: impl AsRef<Path>,
        meta: &Metadata,
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;

        let user = meta.user.clone().and_then(helpers::uid_from_name);
        // use uid from user if valid, else from saved uid (if saved)
        let uid = user.or_else(|| meta.uid.map(Uid::from_raw));

        let group = meta.group.clone().and_then(helpers::gid_from_name);
        // use gid from group if valid, else from saved gid (if saved)
        let gid = group.or_else(|| meta.gid.map(Gid::from_raw));

        fchownat(AT_FDCWD, &filename, uid, gid, AtFlags::AT_SYMLINK_NOFOLLOW)
            .map_err(LocalDestinationErrorKind::FromErrnoError)?;
        Ok(())
    }

    #[cfg(windows)]
    // TODO: Windows support
    /// Set uid/gid for `item` (relative to the base path) utilizing the file metadata
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the uid/gid for
    /// * `meta` - The metadata to get the uid/gid from
    ///
    /// # Errors
    ///
    /// * If the uid/gid could not be set.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn set_uid_gid(
        &self,
        _item: impl AsRef<Path>,
        _meta: &Metadata,
    ) -> LocalDestinationResult<()> {
        Ok(())
    }

    #[cfg(not(windows))]
    /// Set uid/gid for `item` (relative to the base path) utilizing the file metadata
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the uid/gid for
    /// * `meta` - The metadata to get the uid/gid from
    ///
    /// # Errors
    ///
    /// * If the uid/gid could not be set.
    #[allow(clippy::similar_names)]
    pub(crate) fn set_uid_gid(
        &self,
        item: impl AsRef<Path>,
        meta: &Metadata,
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;

        let uid = meta.uid.map(Uid::from_raw);
        let gid = meta.gid.map(Gid::from_raw);

        fchownat(AT_FDCWD, &filename, uid, gid, AtFlags::AT_SYMLINK_NOFOLLOW)
            .map_err(LocalDestinationErrorKind::FromErrnoError)?;
        Ok(())
    }

    #[cfg(windows)]
    // TODO: Windows support
    /// Set permissions for `item` (relative to the base path) from `node`
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the permissions for
    /// * `node` - The node to get the permissions from
    ///
    /// # Errors
    ///
    /// * If the permissions could not be set.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn set_permission(
        &self,
        _item: impl AsRef<Path>,
        _node: &Node,
    ) -> LocalDestinationResult<()> {
        Ok(())
    }

    #[cfg(not(windows))]
    /// Set permissions for `item` (relative to the base path) from `node`
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the permissions for
    /// * `node` - The node to get the permissions from
    ///
    /// # Errors
    ///
    /// * If the permissions could not be set.
    #[allow(clippy::similar_names)]
    pub(crate) fn set_permission(
        &self,
        item: impl AsRef<Path>,
        node: &Node,
    ) -> LocalDestinationResult<()> {
        if node.is_symlink() {
            return Ok(());
        }

        let filename = self.path(item)?;

        if let Some(mode) = node.meta.mode {
            let mode = map_mode_from_go(mode);
            fs::set_permissions(filename, fs::Permissions::from_mode(mode))
                .map_err(LocalDestinationErrorKind::SettingFilePermissionsFailed)?;
        }
        Ok(())
    }

    #[cfg(any(windows, target_os = "openbsd"))]
    // TODO: Windows support
    // TODO: openbsd support
    /// Set extended attributes for `item` (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the extended attributes for
    /// * `extended_attributes` - The extended attributes to set
    ///
    /// # Errors
    ///
    /// * If the extended attributes could not be set.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn set_extended_attributes(
        &self,
        _item: impl AsRef<Path>,
        _extended_attributes: &[ExtendedAttribute],
    ) -> LocalDestinationResult<()> {
        Ok(())
    }

    #[cfg(not(any(windows, target_os = "openbsd")))]
    /// Set extended attributes for `item` (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the extended attributes for
    /// * `extended_attributes` - The extended attributes to set
    ///
    /// # Errors
    ///
    /// * If listing the extended attributes failed.
    /// * If getting an extended attribute failed.
    /// * If setting an extended attribute failed.
    ///
    /// # Returns
    ///
    /// Ok if the extended attributes were set.
    ///
    /// # Panics
    ///
    /// * If the extended attributes could not be set.
    pub(crate) fn set_extended_attributes(
        &self,
        item: impl AsRef<Path>,
        extended_attributes: &[ExtendedAttribute],
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;
        let mut done = vec![false; extended_attributes.len()];

        for curr_name in xattr::list(&filename).map_err(|err| {
            LocalDestinationErrorKind::ListingXattrsFailed {
                source: err,
                path: filename.clone(),
            }
        })? {
            match extended_attributes.iter().enumerate().find(
                |(_, ExtendedAttribute { name, .. })| name == curr_name.to_string_lossy().as_ref(),
            ) {
                Some((index, ExtendedAttribute { name, value })) => {
                    let curr_value = xattr::get(&filename, name).map_err(|err| {
                        LocalDestinationErrorKind::GettingXattrFailed {
                            name: name.clone(),
                            filename: filename.clone(),
                            source: err,
                        }
                    })?;
                    if value != &curr_value {
                        xattr::set(&filename, name, value.as_ref().unwrap_or(&Vec::new()))
                            .map_err(|err| LocalDestinationErrorKind::SettingXattrFailed {
                                name: name.clone(),
                                filename: filename.clone(),
                                source: err,
                            })?;
                    }
                    done[index] = true;
                }
                None => {
                    if let Err(err) = xattr::remove(&filename, &curr_name) {
                        warn!(
                            "error removing xattr {} on {}: {err}",
                            curr_name.display(),
                            filename.display()
                        );
                    }
                }
            }
        }

        for (index, ExtendedAttribute { name, value }) in extended_attributes.iter().enumerate() {
            if !done[index] {
                xattr::set(&filename, name, value.as_ref().unwrap_or(&Vec::new())).map_err(
                    |err| LocalDestinationErrorKind::SettingXattrFailed {
                        name: name.clone(),
                        filename: filename.clone(),
                        source: err,
                    },
                )?;
            }
        }

        Ok(())
    }

    /// Set length of `item` (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to set the length for
    /// * `size` - The size to set the length to
    ///
    /// # Errors
    ///
    /// * If the file does not have a parent.
    /// * If the directory could not be created.
    /// * If the file could not be opened.
    /// * If the length of the file could not be set.
    ///
    /// # Notes
    ///
    /// If the file exists, truncate it to the given length. (TODO: check if this is correct)
    /// If it doesn't exist, create a new (empty) one with given length.
    pub(crate) fn set_length(
        &self,
        item: impl AsRef<Path>,
        size: u64,
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;
        let dir = filename
            .parent()
            .ok_or_else(|| LocalDestinationErrorKind::FileDoesNotHaveParent(filename.clone()))?;
        fs::create_dir_all(dir).map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;

        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(filename)
            .map_err(LocalDestinationErrorKind::OpeningFileFailed)?
            .set_len(size)
            .map_err(LocalDestinationErrorKind::SettingFileLengthFailed)?;
        Ok(())
    }

    #[cfg(windows)]
    // TODO: Windows support
    /// Create a special file (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to create
    /// * `node` - The node to get the type from
    ///
    /// # Errors
    ///
    /// * If the special file could not be created.
    ///
    /// # Returns
    ///
    /// Ok if the special file was created.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn create_special(
        &self,
        _item: impl AsRef<Path>,
        _node: &Node,
    ) -> LocalDestinationResult<()> {
        Ok(())
    }

    #[cfg(not(windows))]
    /// Create a special file (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to create
    /// * `node` - The node to get the type from
    ///
    /// # Errors
    ///
    /// * If the symlink could not be created.
    /// * If the device could not be converted to the correct type.
    /// * If the device could not be created.
    pub(crate) fn create_special(
        &self,
        item: impl AsRef<Path>,
        node: &Node,
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;

        match &node.node_type {
            NodeType::Symlink { .. } => {
                let linktarget = node.node_type.to_link();
                symlink(linktarget, &filename).map_err(|err| {
                    LocalDestinationErrorKind::SymlinkingFailed {
                        linktarget: linktarget.to_path_buf(),
                        filename,
                        source: err,
                    }
                })?;
            }
            NodeType::Dev { device } => {
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "openbsd",
                    all(target_os = "android", target_pointer_width = "32")
                )))]
                let device = *device;
                #[cfg(any(target_os = "macos", target_os = "ios", target_os = "openbsd"))]
                let device = i32::try_from(*device).map_err(|err| {
                    LocalDestinationErrorKind::DeviceIdConversionFailed {
                        target: "i32".to_string(),
                        device: *device,
                        source: err,
                    }
                })?;
                #[cfg(all(target_os = "android", target_pointer_width = "32"))]
                let device = u32::try_from(*device).map_err(|err| {
                    LocalDestinationErrorKind::DeviceIdConversionFailed {
                        target: "u32".to_string(),
                        device: *device,
                        source: err,
                    }
                })?;
                mknod(&filename, SFlag::S_IFBLK, Mode::empty(), device)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Chardev { device } => {
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "openbsd",
                    all(target_os = "android", target_pointer_width = "32")
                )))]
                let device = *device;
                #[cfg(any(target_os = "macos", target_os = "ios", target_os = "openbsd"))]
                let device = i32::try_from(*device).map_err(|err| {
                    LocalDestinationErrorKind::DeviceIdConversionFailed {
                        target: "i32".to_string(),
                        device: *device,
                        source: err,
                    }
                })?;
                #[cfg(all(target_os = "android", target_pointer_width = "32"))]
                let device = u32::try_from(*device).map_err(|err| {
                    LocalDestinationErrorKind::DeviceIdConversionFailed {
                        target: "u32".to_string(),
                        device: *device,
                        source: err,
                    }
                })?;
                mknod(&filename, SFlag::S_IFCHR, Mode::empty(), device)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Fifo => {
                mknod(&filename, SFlag::S_IFIFO, Mode::empty(), 0)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Socket => {
                mknod(&filename, SFlag::S_IFSOCK, Mode::empty(), 0)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Read the given item (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The item to read
    /// * `offset` - The offset to read from
    /// * `length` - The length to read
    ///
    /// # Errors
    ///
    /// * If the file could not be opened.
    /// * If the file could not be sought to the given position.
    /// * If the length of the file could not be converted to u32.
    /// * If the length of the file could not be read.
    pub(crate) fn read_at(
        &self,
        item: impl AsRef<Path>,
        offset: u64,
        length: u64,
    ) -> LocalDestinationResult<Bytes> {
        let filename = self.path(item)?;
        let mut file =
            File::open(filename).map_err(LocalDestinationErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset))
            .map_err(LocalDestinationErrorKind::CouldNotSeekToPositionInFile)?;
        let mut vec = vec![
            0;
            length.try_into().map_err(|err| {
                LocalDestinationErrorKind::LengthConversionFailed {
                    target: "u8".to_string(),
                    length,
                    source: err,
                }
            })?
        ];
        file.read_exact(&mut vec)
            .map_err(LocalDestinationErrorKind::ReadingExactLengthOfFileFailed)?;
        Ok(vec.into())
    }

    /// Check if a matching file exists.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to check
    /// * `size` - The size to check
    ///
    /// # Returns
    ///
    /// If a file exists and size matches, this returns a `File` open for reading.
    /// In all other cases, returns `None`
    pub fn get_matching_file(&self, item: impl AsRef<Path>, size: u64) -> Option<File> {
        let filename = self.path(item).ok()?;
        fs::symlink_metadata(&filename).map_or_else(
            |_| None,
            |meta| {
                if meta.is_file() && meta.len() == size {
                    File::open(&filename).ok()
                } else {
                    None
                }
            },
        )
    }

    /// Write `data` to given item (relative to the base path) at `offset`
    ///
    /// # Arguments
    ///
    /// * `item` - The item to write to
    /// * `offset` - The offset to write at
    /// * `data` - The data to write
    ///
    /// # Errors
    ///
    /// * If the file could not be opened.
    /// * If the file could not be sought to the given position.
    /// * If the bytes could not be written to the file.
    ///
    /// # Notes
    ///
    /// This will create the file if it doesn't exist.
    pub(crate) fn write_at(
        &self,
        item: impl AsRef<Path>,
        offset: u64,
        data: &[u8],
    ) -> LocalDestinationResult<()> {
        let filename = self.path(item)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(filename)
            .map_err(LocalDestinationErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset))
            .map_err(LocalDestinationErrorKind::CouldNotSeekToPositionInFile)?;
        file.write_all(data)
            .map_err(LocalDestinationErrorKind::CouldNotWriteToBuffer)?;
        Ok(())
    }

    /// Create a hardlink `item` pointing to `source_item`, both relative to the base path.
    ///
    /// An entry that already exists at `item` is replaced, unless it already is that hardlink.
    ///
    /// # Arguments
    ///
    /// * `source_item` - The already-restored file to link to
    /// * `item` - The path to create as a hardlink
    ///
    /// # Errors
    ///
    /// * If the new hardlink does not have a parent directory.
    /// * If the directory could not be created.
    /// * If the hardlink could not be created.
    pub(crate) fn hard_link(
        &self,
        source_item: impl AsRef<Path>,
        item: impl AsRef<Path>,
    ) -> LocalDestinationResult<()> {
        let source_path = self.path(source_item)?;
        let filename = self.path(item)?;
        let dir = filename
            .parent()
            .ok_or_else(|| LocalDestinationErrorKind::FileDoesNotHaveParent(filename.clone()))?;
        fs::create_dir_all(dir).map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;
        link_over_existing(&source_path, &filename).map_err(|err| {
            LocalDestinationErrorKind::HardLinkingFailed {
                source_path,
                filename,
                source: err,
            }
        })?;
        Ok(())
    }
}

/// Hardlink `filename` to `source`, replacing a different entry that sits at `filename`.
///
/// # Arguments
///
/// * `source` - The file to link to
/// * `filename` - The path of the hardlink
///
/// # Errors
///
/// * If `filename` could not be inspected, removed or linked.
fn link_over_existing(source: &Path, filename: &Path) -> std::io::Result<()> {
    match fs::hard_link(source, filename) {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let is_symlink = fs::symlink_metadata(filename)?.is_symlink();
            if !is_symlink && is_same_file(source, filename)? {
                return Ok(());
            }
            fs::remove_file(filename)?;
            fs::hard_link(source, filename)
        }
        linked => linked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn hard_link_leaves_a_directory_in_place() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let dest = LocalDestination::new(temp.path().to_str().unwrap(), false, false)?;
        fs::write(temp.path().join("canonical"), "restored")?;
        fs::create_dir(temp.path().join("link"))?;

        assert!(dest.hard_link("canonical", "link").is_err());

        assert!(temp.path().join("link").is_dir());
        Ok(())
    }

    #[test]
    #[cfg(not(windows))]
    fn hard_link_replaces_a_symlink_to_the_canonical_file() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempdir()?;
        let dest = LocalDestination::new(temp.path().to_str().unwrap(), false, false)?;
        fs::write(temp.path().join("canonical"), "restored")?;
        symlink("canonical", temp.path().join("link"))?;

        dest.hard_link("canonical", "link")?;

        assert!(!fs::symlink_metadata(temp.path().join("link"))?.is_symlink());
        Ok(())
    }

    #[test]
    #[cfg(not(windows))]
    fn hard_link_keeps_the_file_when_a_symlinked_directory_makes_both_paths_one()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let dest = LocalDestination::new(temp.path().to_str().unwrap(), false, false)?;
        fs::create_dir(temp.path().join("real"))?;
        symlink("real", temp.path().join("alias"))?;
        fs::write(temp.path().join("real/canonical"), "restored")?;

        dest.hard_link("real/canonical", "alias/canonical")?;

        assert_eq!(
            fs::read_to_string(temp.path().join("real/canonical"))?,
            "restored"
        );
        Ok(())
    }

    #[test]
    fn test_path_sanitization() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let dest = LocalDestination::new(temp.path().to_str().unwrap(), false, false)?;

        // Valid relative paths
        assert_eq!(dest.path("foo/bar")?, temp.path().join("foo/bar"));
        assert_eq!(dest.path("./foo")?, temp.path().join("foo"));
        assert_eq!(dest.path("")?, temp.path());

        // Path traversal attempts
        assert!(dest.path("../escaped").is_err());
        assert!(dest.path("../../etc/passwd").is_err());
        assert!(dest.path("a/../../b").is_err());
        assert!(dest.path("/absolute/path").is_err());

        Ok(())
    }

    #[test]
    fn test_operations_reject_traversal() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let dest = LocalDestination::new(temp.path().to_str().unwrap(), false, false)?;

        assert!(dest.create_dir("../escaped_dir").is_err());
        assert!(dest.write_at("../escaped_file", 0, b"data").is_err());
        assert!(dest.read_at("../escaped_file", 0, 4).is_err());
        assert!(dest.get_matching_file("../escaped_file", 4).is_none());

        Ok(())
    }
}
