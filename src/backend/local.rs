use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    num::TryFromIntError,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use aho_corasick::AhoCorasick;
use anyhow::Result;
use bytes::Bytes;
#[allow(unused_imports)]
use cached::proc_macro::cached;
use displaydoc::Display;
use log::{debug, trace, warn};
use shell_words::split;
use thiserror::Error;
use walkdir::WalkDir;

use crate::{
    backend::{FileType, ReadBackend, WriteBackend, ALL_FILE_TYPES},
    id::Id,
};

#[derive(Clone, Debug)]
pub struct LocalBackend {
    /// The base path of the backend.
    path: PathBuf,
    /// The command to call after a file was created.
    post_create_command: Option<String>,
    /// The command to call after a file was deleted.
    post_delete_command: Option<String>,
}

/// [`LocalErrorKind`] describes the errors that can be returned by an action on the filesystem in Backends
#[derive(Error, Debug, Display)]
pub enum LocalBackendErrorKind {
    /// directory creation failed: `{0:?}`
    DirectoryCreationFailed(#[from] std::io::Error),
    /// querying metadata failed: `{0:?}`
    QueryingMetadataFailed(std::io::Error),
    /// querying WalkDir metadata failed: `{0:?}`
    QueryingWalkDirMetadataFailed(walkdir::Error),
    /// executtion of command failed: `{0:?}`
    CommandExecutionFailed(std::io::Error),
    /// command was not successful for filename {file_name}, type {file_type}, id {id}: {status}
    CommandNotSuccessful {
        file_name: String,
        file_type: String,
        id: String,
        status: ExitStatus,
    },
    /// error building automaton `{0:?}`
    FromAhoCorasick(#[from] aho_corasick::BuildError),
    /// {0:?}
    FromSplitError(#[from] shell_words::ParseError),
    /// {0:?}
    #[error(transparent)]
    FromTryIntError(#[from] TryFromIntError),
    /// {0:?}
    #[error(transparent)]
    FromWalkdirError(#[from] walkdir::Error),
    /// removing file failed: `{0:?}`
    FileRemovalFailed(std::io::Error),
    /// opening file failed: `{0:?}`
    OpeningFileFailed(std::io::Error),
    /// setting file length failed: `{0:?}`
    SettingFileLengthFailed(std::io::Error),
    /// can't jump to position in file: `{0:?}`
    CouldNotSeekToPositionInFile(std::io::Error),
    /// couldn't write to buffer: `{0:?}`
    CouldNotWriteToBuffer(std::io::Error),
    /// reading file contents failed: `{0:?}`
    ReadingContentsOfFileFailed(std::io::Error),
    /// reading exact length of file contents failed: `{0:?}`
    ReadingExactLengthOfFileFailed(std::io::Error),
    /// failed to sync OS Metadata to disk: `{0:?}`
    SyncingOfOsMetadataFailed(std::io::Error),
}

impl LocalBackend {
    /// Create a new [`LocalBackend`]
    ///
    /// # Arguments
    ///
    /// * `path` - The base path of the backend
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// [`LocalErrorKind::DirectoryCreationFailed`]: crate::error::LocalErrorKind::DirectoryCreationFailed
    // TODO: We should use `impl Into<Path/PathBuf>` here. we even use it in the body!
    pub fn new(path: &str, options: impl IntoIterator<Item = (String, String)>) -> Result<Self> {
        let path = path.into();
        fs::create_dir_all(&path).map_err(LocalBackendErrorKind::DirectoryCreationFailed)?;
        let mut post_create_command = None;
        let mut post_delete_command = None;
        for (option, value) in options {
            match option.as_str() {
                "post-create-command" => {
                    post_create_command = Some(value);
                }
                "post-delete-command" => {
                    post_delete_command = Some(value);
                }
                opt => {
                    warn!("Option {opt} is not supported! Ignoring it.");
                }
            }
        }
        Ok(Self {
            path,
            post_create_command,
            post_delete_command,
        })
    }

    /// Path to the given file type and id.
    ///
    /// If the file type is `FileType::Pack`, the id will be used to determine the subdirectory.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Returns
    ///
    /// The path to the file.
    fn path(&self, tpe: FileType, id: &Id) -> PathBuf {
        let hex_id = id.to_hex();
        match tpe {
            FileType::Config => self.path.join("config"),
            FileType::Pack => self.path.join("data").join(&hex_id[0..2]).join(hex_id),
            _ => self.path.join(tpe.dirname()).join(hex_id),
        }
    }

    /// Call the given command.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `filename` - The path to the file.
    /// * `command` - The command to call.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::FromAhoCorasick`] - If the patterns could not be compiled.
    /// * [`LocalErrorKind::FromSplitError`] - If the command could not be parsed.
    /// * [`LocalErrorKind::CommandExecutionFailed`] - If the command could not be executed.
    /// * [`LocalErrorKind::CommandNotSuccessful`] - If the command was not successful.
    ///
    /// # Notes
    ///
    /// The following placeholders are supported:
    /// * `%file` - The path to the file.
    /// * `%type` - The type of the file.
    /// * `%id` - The id of the file.
    ///
    /// [`LocalErrorKind::FromAhoCorasick`]: crate::error::LocalErrorKind::FromAhoCorasick
    /// [`LocalErrorKind::FromSplitError`]: crate::error::LocalErrorKind::FromSplitError
    /// [`LocalErrorKind::CommandExecutionFailed`]: crate::error::LocalErrorKind::CommandExecutionFailed
    /// [`LocalErrorKind::CommandNotSuccessful`]: crate::error::LocalErrorKind::CommandNotSuccessful
    fn call_command(tpe: FileType, id: &Id, filename: &Path, command: &str) -> Result<()> {
        let id = id.to_hex();
        let patterns = &["%file", "%type", "%id"];
        let ac = AhoCorasick::new(patterns).map_err(LocalBackendErrorKind::FromAhoCorasick)?;
        let replace_with = &[filename.to_str().unwrap(), tpe.dirname(), id.as_str()];
        let actual_command = ac.replace_all(command, replace_with);
        debug!("calling {actual_command}...");
        let commands = split(&actual_command).map_err(LocalBackendErrorKind::FromSplitError)?;
        let status = Command::new(&commands[0])
            .args(&commands[1..])
            .status()
            .map_err(LocalBackendErrorKind::CommandExecutionFailed)?;
        if !status.success() {
            return Err(LocalBackendErrorKind::CommandNotSuccessful {
                file_name: replace_with[0].to_owned(),
                file_type: replace_with[1].to_owned(),
                id: replace_with[2].to_owned(),
                status,
            }
            .into());
        }
        Ok(())
    }
}

impl ReadBackend for LocalBackend {
    /// Returns the location of the backend.
    ///
    /// This is `local:<path>`.
    fn location(&self) -> String {
        let mut location = "local:".to_string();
        location.push_str(&self.path.to_string_lossy());
        location
    }

    /// Lists all files of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    ///
    /// # Notes
    ///
    /// If the file type is `FileType::Config`, this will return a list with a single default id.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    fn list(&self, tpe: FileType) -> Result<Vec<Id>> {
        trace!("listing tpe: {tpe:?}");
        if tpe == FileType::Config {
            return Ok(if self.path.join("config").exists() {
                vec![Id::default()]
            } else {
                Vec::new()
            });
        }

        let walker = WalkDir::new(self.path.join(tpe.dirname()))
            .into_iter()
            .filter_map(walkdir::Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| Id::from_hex(&e.file_name().to_string_lossy()))
            .filter_map(std::result::Result::ok);
        Ok(walker.collect())
    }

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::QueryingMetadataFailed`] - If the metadata of the file could not be queried.
    /// * [`LocalErrorKind::FromTryIntError`] - If the length of the file could not be converted to u32.
    /// * [`LocalErrorKind::QueryingWalkDirMetadataFailed`] - If the metadata of the file could not be queried.
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    ///
    /// [`LocalErrorKind::QueryingMetadataFailed`]: crate::error::LocalErrorKind::QueryingMetadataFailed
    /// [`LocalErrorKind::FromTryIntError`]: crate::error::LocalErrorKind::FromTryIntError
    /// [`LocalErrorKind::QueryingWalkDirMetadataFailed`]: crate::error::LocalErrorKind::QueryingWalkDirMetadataFailed
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        trace!("listing tpe: {tpe:?}");
        let path = self.path.join(tpe.dirname());

        if tpe == FileType::Config {
            return Ok(if path.exists() {
                vec![(
                    Id::default(),
                    path.metadata()
                        .map_err(LocalBackendErrorKind::QueryingMetadataFailed)?
                        .len()
                        .try_into()
                        .map_err(LocalBackendErrorKind::FromTryIntError)?,
                )]
            } else {
                Vec::new()
            });
        }

        let walker = WalkDir::new(path)
            .into_iter()
            .filter_map(walkdir::Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| -> Result<_> {
                Ok((
                    Id::from_hex(&e.file_name().to_string_lossy())?,
                    e.metadata()
                        .map_err(LocalBackendErrorKind::QueryingWalkDirMetadataFailed)?
                        .len()
                        .try_into()
                        .map_err(LocalBackendErrorKind::FromTryIntError)?,
                ))
            })
            .filter_map(Result::ok);

        Ok(walker.collect())
    }

    /// Reads full data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::ReadingContentsOfFileFailed`] - If the file could not be read.
    ///
    /// [`LocalErrorKind::ReadingContentsOfFileFailed`]: crate::error::LocalErrorKind::ReadingContentsOfFileFailed
    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");
        Ok(fs::read(self.path(tpe, id))
            .map_err(LocalBackendErrorKind::ReadingContentsOfFileFailed)?
            .into())
    }

    /// Reads partial data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalErrorKind::CouldNotSeekToPositionInFile`] - If the file could not be seeked to the given position.
    /// * [`LocalErrorKind::FromTryIntError`] - If the length of the file could not be converted to u32.
    /// * [`LocalErrorKind::ReadingExactLengthOfFileFailed`] - If the length of the file could not be read.
    ///
    /// [`LocalErrorKind::OpeningFileFailed`]: crate::error::LocalErrorKind::OpeningFileFailed
    /// [`LocalErrorKind::CouldNotSeekToPositionInFile`]: crate::error::LocalErrorKind::CouldNotSeekToPositionInFile
    /// [`LocalErrorKind::FromTryIntError`]: crate::error::LocalErrorKind::FromTryIntError
    /// [`LocalErrorKind::ReadingExactLengthOfFileFailed`]: crate::error::LocalErrorKind::ReadingExactLengthOfFileFailed
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let mut file =
            File::open(self.path(tpe, id)).map_err(LocalBackendErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset.into()))
            .map_err(LocalErrorKind::CouldNotSeekToPositionInFile)?;
        let mut vec = vec![0; length.try_into().map_err(LocalErrorKind::FromTryIntError)?];
        file.read_exact(&mut vec)
            .map_err(LocalBackendErrorKind::ReadingExactLengthOfFileFailed)?;
        Ok(vec.into())
    }
}

impl WriteBackend for LocalBackend {
    /// Create a repository on the backend.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// [`LocalErrorKind::DirectoryCreationFailed`]: crate::error::LocalErrorKind::DirectoryCreationFailed
    fn create(&self) -> Result<()> {
        trace!("creating repo at {:?}", self.path);

        for tpe in ALL_FILE_TYPES {
            fs::create_dir_all(self.path.join(tpe.dirname()))
                .map_err(LocalBackendErrorKind::DirectoryCreationFailed)?;
        }
        for i in 0u8..=255 {
            fs::create_dir_all(self.path.join("data").join(hex::encode([i])))
                .map_err(LocalBackendErrorKind::DirectoryCreationFailed)?;
        }
        Ok(())
    }

    /// Write the given bytes to the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    /// * `buf` - The bytes to write.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalErrorKind::FromTryIntError`] - If the length of the bytes could not be converted to u64.
    /// * [`LocalErrorKind::SettingFileLengthFailed`] - If the length of the file could not be set.
    /// * [`LocalErrorKind::CouldNotWriteToBuffer`] - If the bytes could not be written to the file.
    /// * [`LocalErrorKind::SyncingOfOsMetadataFailed`] - If the metadata of the file could not be synced.
    ///
    /// [`LocalErrorKind::OpeningFileFailed`]: crate::error::LocalErrorKind::OpeningFileFailed
    /// [`LocalErrorKind::FromTryIntError`]: crate::error::LocalErrorKind::FromTryIntError
    /// [`LocalErrorKind::SettingFileLengthFailed`]: crate::error::LocalErrorKind::SettingFileLengthFailed
    /// [`LocalErrorKind::CouldNotWriteToBuffer`]: crate::error::LocalErrorKind::CouldNotWriteToBuffer
    /// [`LocalErrorKind::SyncingOfOsMetadataFailed`]: crate::error::LocalErrorKind::SyncingOfOsMetadataFailed
    fn write_bytes(&self, tpe: FileType, id: &Id, _cacheable: bool, buf: Bytes) -> Result<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&filename)
            .map_err(LocalBackendErrorKind::OpeningFileFailed)?;
        file.set_len(
            buf.len()
                .try_into()
                .map_err(LocalBackendErrorKind::FromTryIntError)?,
        )
        .map_err(LocalBackendErrorKind::SettingFileLengthFailed)?;
        file.write_all(&buf)
            .map_err(LocalBackendErrorKind::CouldNotWriteToBuffer)?;
        file.sync_all()
            .map_err(LocalBackendErrorKind::SyncingOfOsMetadataFailed)?;
        if let Some(command) = &self.post_create_command {
            if let Err(err) = Self::call_command(tpe, id, &filename, command) {
                warn!("post-create: {err}");
            }
        }
        Ok(())
    }

    /// Remove the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    ///
    /// # Errors
    ///
    /// * [`LocalErrorKind::FileRemovalFailed`] - If the file could not be removed.
    ///
    /// [`LocalErrorKind::FileRemovalFailed`]: crate::error::LocalErrorKind::FileRemovalFailed
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> Result<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        fs::remove_file(&filename).map_err(LocalBackendErrorKind::FileRemovalFailed)?;
        if let Some(command) = &self.post_delete_command {
            if let Err(err) = Self::call_command(tpe, id, &filename, command) {
                warn!("post-delete: {err}");
            }
        }
        Ok(())
    }
}
