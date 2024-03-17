#[cfg(not(windows))]
use std::os::unix::fs::{symlink, PermissionsExt};

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use bytes::Bytes;
#[allow(unused_imports)]
use cached::proc_macro::cached;
use filetime::{set_symlink_file_times, FileTime};
#[cfg(not(windows))]
use log::warn;
#[cfg(not(windows))]
use nix::sys::stat::{mknod, Mode, SFlag};
#[cfg(not(windows))]
use nix::{
    fcntl::AtFlags,
    unistd::{fchownat, Gid, Group, Uid, User},
};

#[cfg(not(windows))]
use crate::backend::ignore::mapper::map_mode_from_go;
#[cfg(not(windows))]
use crate::backend::node::NodeType;
use crate::{
    backend::node::{ExtendedAttribute, Metadata, Node},
    error::LocalDestinationErrorKind,
    RusticResult,
};

#[derive(Clone, Debug)]
/// Local destination, used when restoring.
pub struct LocalDestination {
    /// The base path of the destination.
    path: PathBuf,
    /// Whether we expect a single file as destination.
    is_file: bool,
}

// Helper function to cache mapping user name -> uid
#[cfg(not(windows))]
#[cached]
fn uid_from_name(name: String) -> Option<Uid> {
    User::from_name(&name).ok()?.map(|u| u.uid)
}

// Helper function to cache mapping group name -> gid
#[cfg(not(windows))]
#[cached]
fn gid_from_name(name: String) -> Option<Gid> {
    Group::from_name(&name).ok()?.map(|g| g.gid)
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
    /// * [`LocalDestinationErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// [`LocalDestinationErrorKind::DirectoryCreationFailed`]: crate::error::LocalDestinationErrorKind::DirectoryCreationFailed
    pub fn new(path: &str, create: bool, expect_file: bool) -> RusticResult<Self> {
        let is_dir = path.ends_with('/');
        let path = PathBuf::from(path);
        let is_file = path.is_file() || (!path.is_dir() && !is_dir && expect_file);

        if create {
            if is_file {
                if let Some(path) = path.parent() {
                    fs::create_dir_all(path)
                        .map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;
                }
            } else {
                fs::create_dir_all(&path)
                    .map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;
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
    pub(crate) fn path_of(&self, item: impl AsRef<Path>) -> PathBuf {
        if self.is_file {
            self.path.clone()
        } else {
            self.path.join(item)
        }
    }

    /// Remove the given directory (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `dirname` - The directory to remove
    ///
    /// # Errors
    ///
    /// * [`LocalDestinationErrorKind::DirectoryRemovalFailed`] - If the directory could not be removed.
    ///
    /// # Notes
    ///
    /// This will remove the directory recursively.
    ///
    /// [`LocalDestinationErrorKind::DirectoryRemovalFailed`]: crate::error::LocalDestinationErrorKind::DirectoryRemovalFailed
    pub fn remove_dir(&self, dir_name: impl AsRef<Path>) -> RusticResult<()> {
        Ok(fs::remove_dir_all(self.path_of(dir_name))
            .map_err(LocalDestinationErrorKind::DirectoryRemovalFailed)?)
    }

    /// Remove the given file (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `file_name` - The file to remove
    ///
    /// # Errors
    ///
    /// * [`LocalDestinationErrorKind::FileRemovalFailed`] - If the file could not be removed.
    ///
    /// # Notes
    ///
    /// This will remove the file.
    ///
    /// * If the file is a symlink, the symlink will be removed, not the file it points to.
    /// * If the file is a directory or device, this will fail.
    ///
    /// [`LocalDestinationErrorKind::FileRemovalFailed`]: crate::error::LocalDestinationErrorKind::FileRemovalFailed
    pub fn remove_file(&self, file_name: impl AsRef<Path>) -> RusticResult<()> {
        Ok(fs::remove_file(self.path_of(file_name))
            .map_err(LocalDestinationErrorKind::FileRemovalFailed)?)
    }

    /// Create the given directory (relative to the base path)
    ///
    /// # Arguments
    ///
    /// * `item` - The directory to create
    ///
    /// # Errors
    ///
    /// * [`LocalDestinationErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// # Notes
    ///
    /// This will create the directory structure recursively.
    ///
    /// [`LocalDestinationErrorKind::DirectoryCreationFailed`]: crate::error::LocalDestinationErrorKind::DirectoryCreationFailed
    pub fn create_dir(&self, item: impl AsRef<Path>) -> RusticResult<()> {
        fs::create_dir_all(self.path_of(item))
            .map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;
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
    /// * [`LocalDestinationErrorKind::SettingTimeMetadataFailed`] - If the times could not be set
    ///
    /// [`LocalDestinationErrorKind::SettingTimeMetadataFailed`]: crate::error::LocalDestinationErrorKind::SettingTimeMetadataFailed
    pub fn set_times(&self, item: impl AsRef<Path>, meta: &Metadata) -> RusticResult<()> {
        let file_name = self.path_of(item);
        if let Some(mtime) = meta.mtime {
            let atime = meta.atime.unwrap_or(mtime);
            set_symlink_file_times(
                file_name,
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
    /// If the user/group could not be set.
    pub fn set_user_group(&self, _item: impl AsRef<Path>, _meta: &Metadata) -> RusticResult<()> {
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
    /// * [`LocalDestinationErrorKind::FromErrnoError`] - If the user/group could not be set.
    ///
    /// [`LocalDestinationErrorKind::FromErrnoError`]: crate::error::LocalDestinationErrorKind::FromErrnoError
    #[allow(clippy::similar_names)]
    pub fn set_user_group(&self, item: impl AsRef<Path>, meta: &Metadata) -> RusticResult<()> {
        let file_name = self.path_of(item);

        let user = meta.user.clone().and_then(uid_from_name);
        // use uid from user if valid, else from saved uid (if saved)
        let uid = user.or_else(|| meta.uid.map(Uid::from_raw));

        let group = meta.group.clone().and_then(gid_from_name);
        // use gid from group if valid, else from saved gid (if saved)
        let gid = group.or_else(|| meta.gid.map(Gid::from_raw));

        fchownat(None, &file_name, uid, gid, AtFlags::AT_SYMLINK_NOFOLLOW)
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
    /// If the uid/gid could not be set.
    pub fn set_uid_gid(&self, _item: impl AsRef<Path>, _meta: &Metadata) -> RusticResult<()> {
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
    /// * [`LocalDestinationErrorKind::FromErrnoError`] - If the uid/gid could not be set.
    ///
    /// [`LocalDestinationErrorKind::FromErrnoError`]: crate::error::LocalDestinationErrorKind::FromErrnoError
    #[allow(clippy::similar_names)]
    pub fn set_uid_gid(&self, item: impl AsRef<Path>, meta: &Metadata) -> RusticResult<()> {
        let file_name = self.path_of(item);

        let uid = meta.uid.map(Uid::from_raw);
        let gid = meta.gid.map(Gid::from_raw);

        fchownat(None, &file_name, uid, gid, AtFlags::AT_SYMLINK_NOFOLLOW)
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
    /// If the permissions could not be set.
    pub fn set_permission(&self, _item: impl AsRef<Path>, _node: &Node) -> RusticResult<()> {
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
    /// * [`LocalDestinationErrorKind::SettingFilePermissionsFailed`] - If the permissions could not be set.
    ///
    /// [`LocalDestinationErrorKind::SettingFilePermissionsFailed`]: crate::error::LocalDestinationErrorKind::SettingFilePermissionsFailed
    #[allow(clippy::similar_names)]
    pub fn set_permission(&self, item: impl AsRef<Path>, node: &Node) -> RusticResult<()> {
        if node.is_symlink() {
            return Ok(());
        }

        let file_name = self.path_of(item);

        if let Some(mode) = node.meta.mode {
            let mode = map_mode_from_go(mode);
            std::fs::set_permissions(file_name, fs::Permissions::from_mode(mode))
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
    /// If the extended attributes could not be set.
    pub fn set_extended_attributes(
        &self,
        _item: impl AsRef<Path>,
        _extended_attributes: &[ExtendedAttribute],
    ) -> RusticResult<()> {
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
    /// * [`LocalDestinationErrorKind::ListingXattrsFailed`] - If listing the extended attributes failed.
    /// * [`LocalDestinationErrorKind::GettingXattrFailed`] - If getting an extended attribute failed.
    /// * [`LocalDestinationErrorKind::SettingXattrFailed`] - If setting an extended attribute failed.
    ///
    /// [`LocalDestinationErrorKind::ListingXattrsFailed`]: crate::error::LocalDestinationErrorKind::ListingXattrsFailed
    /// [`LocalDestinationErrorKind::GettingXattrFailed`]: crate::error::LocalDestinationErrorKind::GettingXattrFailed
    /// [`LocalDestinationErrorKind::SettingXattrFailed`]: crate::error::LocalDestinationErrorKind::SettingXattrFailed
    ///
    /// # Returns
    ///
    /// Ok if the extended attributes were set.
    ///
    /// # Panics
    ///
    /// If the extended attributes could not be set.
    pub fn set_extended_attributes(
        &self,
        item: impl AsRef<Path>,
        extended_attributes: &[ExtendedAttribute],
    ) -> RusticResult<()> {
        let file_name = self.path_of(item);

        for curr_name in xattr::list(&file_name)
            .map_err(|err| LocalDestinationErrorKind::ListingXattrsFailed(err, file_name.clone()))?
        {
            match extended_attributes.iter().enumerate().find(
                |(_, ExtendedAttribute { name, .. })| name == curr_name.to_string_lossy().as_ref(),
            ) {
                Some((index, ExtendedAttribute { name, value })) => {
                    if let Some(curr_value) = xattr::get(&file_name, name).map_err(|err| {
                        LocalDestinationErrorKind::GettingXattrFailed {
                            name: name.clone(),
                            file_name: file_name.clone(),
                            source: err,
                        }
                    })? {
                        if value != &curr_value {
                            xattr::set(&file_name, name, value).map_err(|err| {
                                LocalDestinationErrorKind::SettingXattrFailed {
                                    name: name.clone(),
                                    file_name: file_name.clone(),
                                    source: err,
                                }
                            })?;
                }
                None => {
                    if let Err(err) = xattr::remove(&file_name, &curr_name) {
                        warn!("error removing xattr {curr_name:?} on {file_name:?}: {err}");
                    }
                }
            }
        }

        for (index, ExtendedAttribute { name, value }) in extended_attributes.iter().enumerate() {
            if !successful[index] {
                xattr::set(&file_name, name, value).map_err(|err| {
                    LocalDestinationErrorKind::SettingXattrFailed {
                        name: name.clone(),
                        file_name: file_name.clone(),
                        source: err,
                    }
                })?;
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
    /// * [`LocalDestinationErrorKind::FileDoesNotHaveParent`] - If the file does not have a parent.
    /// * [`LocalDestinationErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    /// * [`LocalDestinationErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalDestinationErrorKind::SettingFileLengthFailed`] - If the length of the file could not be set.
    ///
    /// # Notes
    ///
    /// If the file exists, truncate it to the given length. (TODO: check if this is correct)
    /// If it doesn't exist, create a new (empty) one with given length.
    ///
    /// [`LocalDestinationErrorKind::FileDoesNotHaveParent`]: crate::error::LocalDestinationErrorKind::FileDoesNotHaveParent
    /// [`LocalDestinationErrorKind::DirectoryCreationFailed`]: crate::error::LocalDestinationErrorKind::DirectoryCreationFailed
    /// [`LocalDestinationErrorKind::OpeningFileFailed`]: crate::error::LocalDestinationErrorKind::OpeningFileFailed
    /// [`LocalDestinationErrorKind::SettingFileLengthFailed`]: crate::error::LocalDestinationErrorKind::SettingFileLengthFailed
    pub fn set_length(&self, item: impl AsRef<Path>, size: u64) -> RusticResult<()> {
        let file_name = self.path_of(item);
        let dir = file_name
            .parent()
            .ok_or_else(|| LocalDestinationErrorKind::FileDoesNotHaveParent(file_name.clone()))?;
        fs::create_dir_all(dir).map_err(LocalDestinationErrorKind::DirectoryCreationFailed)?;

        OpenOptions::new()
            .create(true)
            .write(true)
            .open(file_name)
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
    /// If the special file could not be created.
    ///
    /// # Returns
    ///
    /// Ok if the special file was created.
    pub fn create_special(&self, _item: impl AsRef<Path>, _node: &Node) -> RusticResult<()> {
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
    /// * [`LocalDestinationErrorKind::SymlinkingFailed`] - If the symlink could not be created.
    /// * [`LocalDestinationErrorKind::FromTryIntError`] - If the device could not be converted to the correct type.
    /// * [`LocalDestinationErrorKind::FromErrnoError`] - If the device could not be created.
    ///
    /// [`LocalDestinationErrorKind::SymlinkingFailed`]: crate::error::LocalDestinationErrorKind::SymlinkingFailed
    /// [`LocalDestinationErrorKind::FromTryIntError`]: crate::error::LocalDestinationErrorKind::FromTryIntError
    /// [`LocalDestinationErrorKind::FromErrnoError`]: crate::error::LocalDestinationErrorKind::FromErrnoError
    pub fn create_special(&self, item: impl AsRef<Path>, node: &Node) -> RusticResult<()> {
        let file_name = self.path_of(item);

        match &node.node_type {
            NodeType::Symlink { .. } => {
                let link_target = node.node_type.to_link()?;
                symlink(link_target, &file_name).map_err(|err| {
                    LocalDestinationErrorKind::SymlinkingFailed {
                        link_target: link_target.to_path_buf(),
                        file_name,
                        source: err,
                    }
                })?;
            }
            NodeType::Dev { device } => {
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "openbsd",
                    target_os = "freebsd"
                )))]
                let device = *device;
                #[cfg(any(target_os = "macos", target_os = "openbsd"))]
                let device =
                    i32::try_from(*device).map_err(LocalDestinationErrorKind::FromTryIntError)?;
                #[cfg(target_os = "freebsd")]
                let device =
                    u32::try_from(*device).map_err(LocalDestinationErrorKind::FromTryIntError)?;
                mknod(&file_name, SFlag::S_IFBLK, Mode::empty(), device)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Chardev { device } => {
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "openbsd",
                    target_os = "freebsd"
                )))]
                let device = *device;
                #[cfg(any(target_os = "macos", target_os = "openbsd"))]
                let device =
                    i32::try_from(*device).map_err(LocalDestinationErrorKind::FromTryIntError)?;
                #[cfg(target_os = "freebsd")]
                let device =
                    u32::try_from(*device).map_err(LocalDestinationErrorKind::FromTryIntError)?;
                mknod(&file_name, SFlag::S_IFCHR, Mode::empty(), device)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Fifo => {
                mknod(&file_name, SFlag::S_IFIFO, Mode::empty(), 0)
                    .map_err(LocalDestinationErrorKind::FromErrnoError)?;
            }
            NodeType::Socket => {
                mknod(&file_name, SFlag::S_IFSOCK, Mode::empty(), 0)
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
    /// * [`LocalDestinationErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalDestinationErrorKind::CouldNotSeekToPositionInFile`] - If the file could not be seeked to the given position.
    /// * [`LocalDestinationErrorKind::FromTryIntError`] - If the length of the file could not be converted to u32.
    /// * [`LocalDestinationErrorKind::ReadingExactLengthOfFileFailed`] - If the length of the file could not be read.
    ///
    /// [`LocalDestinationErrorKind::OpeningFileFailed`]: crate::error::LocalDestinationErrorKind::OpeningFileFailed
    /// [`LocalDestinationErrorKind::CouldNotSeekToPositionInFile`]: crate::error::LocalDestinationErrorKind::CouldNotSeekToPositionInFile
    /// [`LocalDestinationErrorKind::FromTryIntError`]: crate::error::LocalDestinationErrorKind::FromTryIntError
    /// [`LocalDestinationErrorKind::ReadingExactLengthOfFileFailed`]: crate::error::LocalDestinationErrorKind::ReadingExactLengthOfFileFailed
    pub fn read_at(&self, item: impl AsRef<Path>, offset: u64, length: u64) -> RusticResult<Bytes> {
        let file_name = self.path_of(item);
        let mut file =
            File::open(file_name).map_err(LocalDestinationErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset))
            .map_err(LocalDestinationErrorKind::CouldNotSeekToPositionInFile)?;
        let mut vec = vec![
            0;
            length
                .try_into()
                .map_err(LocalDestinationErrorKind::FromTryIntError)?
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
        let file_name = self.path_of(item);
        fs::symlink_metadata(&file_name).map_or_else(
            |_| None,
            |meta| {
                if meta.is_file() && meta.len() == size {
                    File::open(&file_name).ok()
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
    /// * [`LocalDestinationErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalDestinationErrorKind::CouldNotSeekToPositionInFile`] - If the file could not be seeked to the given position.
    /// * [`LocalDestinationErrorKind::CouldNotWriteToBuffer`] - If the bytes could not be written to the file.
    ///
    /// # Notes
    ///
    /// This will create the file if it doesn't exist.
    ///
    /// [`LocalDestinationErrorKind::OpeningFileFailed`]: crate::error::LocalDestinationErrorKind::OpeningFileFailed
    /// [`LocalDestinationErrorKind::CouldNotSeekToPositionInFile`]: crate::error::LocalDestinationErrorKind::CouldNotSeekToPositionInFile
    /// [`LocalDestinationErrorKind::CouldNotWriteToBuffer`]: crate::error::LocalDestinationErrorKind::CouldNotWriteToBuffer
    pub fn write_at(&self, item: impl AsRef<Path>, offset: u64, data: &[u8]) -> RusticResult<()> {
        let file_name = self.path_of(item);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(file_name)
            .map_err(LocalDestinationErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset))
            .map_err(LocalDestinationErrorKind::CouldNotSeekToPositionInFile)?;
        file.write_all(data)
            .map_err(LocalDestinationErrorKind::CouldNotWriteToBuffer)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {

    use super::*;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn local_destination() -> LocalDestination {
        let temp_dir = TempDir::new().unwrap();

        let dest =
            LocalDestination::new(temp_dir.path().to_string_lossy().as_ref(), true, false).unwrap();

        assert_eq!(dest.path, temp_dir.path());
        assert!(!dest.is_file);

        dest
    }

    #[rstest]
    fn test_create_remove_dir_passes(local_destination: LocalDestination) {
        let dir = "test_dir";

        local_destination.create_dir(dir).unwrap();

        assert!(local_destination.path_of(dir).is_dir());

        local_destination.remove_dir(dir).unwrap();

        assert!(!local_destination.path_of(dir).exists());
    }

    #[rstest]
    #[cfg(not(windows))]
    fn test_uid_from_name_passes() {
        let uid = uid_from_name("root".to_string()).unwrap();
        assert_eq!(uid, Uid::from_raw(0));
    }

    #[rstest]
    #[cfg(not(any(windows, darwin)))]
    fn test_gid_from_name_passes() {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "linux")] {
                let gid = gid_from_name("root".to_string()).unwrap();
                assert_eq!(gid, Gid::from_raw(0));
            }
        }
    }

    // TODO: create_special not implemented yet for win
    // #[rstest]
    // fn test_create_remove_file_passes(local_destination: LocalDestination) {
    //     let file = "test_file";

    //     local_destination
    //         .create_special(
    //             file,
    //             &Node::new(
    //                 file.to_string(),
    //                 NodeType::File,
    //                 Metadata::default(),
    //                 None,
    //                 None,
    //             ),
    //         )
    //         .unwrap();

    //     assert!(local_destination.path_of(file).is_file());

    //     local_destination.remove_file(file).unwrap();

    //     assert!(!local_destination.path_of(file).exists());
    // }
}
