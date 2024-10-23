use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
};

use aho_corasick::AhoCorasick;
use bytes::Bytes;
use log::{debug, trace, warn};
use walkdir::WalkDir;

use rustic_core::{
    CommandInput, FileType, Id, ReadBackend, RusticResult, WriteBackend, ALL_FILE_TYPES,
};

/// [`LocalBackendErrorKind`] describes the errors that can be returned by an action on the filesystem in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum LocalBackendErrorKind {
    /// directory creation failed: `{0:?}`
    DirectoryCreationFailed(std::io::Error),
    /// querying metadata failed: `{0:?}`
    QueryingMetadataFailed(std::io::Error),
    /// querying WalkDir metadata failed: `{0:?}`
    QueryingWalkDirMetadataFailed(walkdir::Error),
    /// execution of command failed: `{0:?}`
    CommandExecutionFailed(std::io::Error),
    /// command was not successful for filename {file_name}, type {file_type}, id {id}: {status}
    CommandNotSuccessful {
        /// File name
        file_name: String,
        /// File type
        file_type: String,
        /// Item ID
        id: String,
        /// Exit status
        status: ExitStatus,
    },
    /// error building automaton `{0:?}`
    FromAhoCorasick(aho_corasick::BuildError),
    /// {0:?}
    #[error(transparent)]
    FromTryIntError(TryFromIntError),
    /// {0:?}
    #[error(transparent)]
    FromWalkdirError(walkdir::Error),
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

/// A local backend.
#[derive(Clone, Debug)]
pub struct LocalBackend {
    /// The base path of the backend.
    path: PathBuf,
    /// The command to call after a file was created.
    post_create_command: Option<String>,
    /// The command to call after a file was deleted.
    post_delete_command: Option<String>,
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
    /// * [`LocalBackendErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// [`LocalBackendErrorKind::DirectoryCreationFailed`]: LocalBackendErrorKind::DirectoryCreationFailed
    pub fn new(
        path: impl AsRef<str>,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> RusticResult<Self> {
        let path = path.as_ref().into();
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
    /// * [`LocalBackendErrorKind::FromAhoCorasick`] - If the patterns could not be compiled.
    /// * [`LocalBackendErrorKind::FromSplitError`] - If the command could not be parsed.
    /// * [`LocalBackendErrorKind::CommandExecutionFailed`] - If the command could not be executed.
    /// * [`LocalBackendErrorKind::CommandNotSuccessful`] - If the command was not successful.
    ///
    /// # Notes
    ///
    /// The following placeholders are supported:
    /// * `%file` - The path to the file.
    /// * `%type` - The type of the file.
    /// * `%id` - The id of the file.
    ///
    /// [`LocalBackendErrorKind::FromAhoCorasick`]: LocalBackendErrorKind::FromAhoCorasick
    /// [`LocalBackendErrorKind::FromSplitError`]: LocalBackendErrorKind::FromSplitError
    /// [`LocalBackendErrorKind::CommandExecutionFailed`]: LocalBackendErrorKind::CommandExecutionFailed
    /// [`LocalBackendErrorKind::CommandNotSuccessful`]: LocalBackendErrorKind::CommandNotSuccessful
    fn call_command(tpe: FileType, id: &Id, filename: &Path, command: &str) -> RusticResult<()> {
        let id = id.to_hex();
        let patterns = &["%file", "%type", "%id"];
        let ac = AhoCorasick::new(patterns).map_err(LocalBackendErrorKind::FromAhoCorasick)?;
        let replace_with = &[filename.to_str().unwrap(), tpe.dirname(), id.as_str()];
        let actual_command = ac.replace_all(command, replace_with);
        debug!("calling {actual_command}...");
        let command: CommandInput = actual_command.parse()?;
        let status = Command::new(command.command())
            .args(command.args())
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
    /// # Notes
    ///
    /// If the file type is `FileType::Config`, this will return a list with a single default id.
    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
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
            .filter_map(|e| e.file_name().to_string_lossy().parse::<Id>().ok());
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
    /// * [`LocalBackendErrorKind::QueryingMetadataFailed`] - If the metadata of the file could not be queried.
    /// * [`LocalBackendErrorKind::FromTryIntError`] - If the length of the file could not be converted to u32.
    /// * [`LocalBackendErrorKind::QueryingWalkDirMetadataFailed`] - If the metadata of the file could not be queried.
    ///
    /// [`LocalBackendErrorKind::QueryingMetadataFailed`]: LocalBackendErrorKind::QueryingMetadataFailed
    /// [`LocalBackendErrorKind::FromTryIntError`]: LocalBackendErrorKind::FromTryIntError
    /// [`LocalBackendErrorKind::QueryingWalkDirMetadataFailed`]: LocalBackendErrorKind::QueryingWalkDirMetadataFailed
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
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
            .map(|e| -> RusticResult<_> {
                Ok((
                    e.file_name().to_string_lossy().parse()?,
                    e.metadata()
                        .map_err(LocalBackendErrorKind::QueryingWalkDirMetadataFailed)
                        .map_err(|_err| todo!("Error transition"))?
                        .len()
                        .try_into()
                        .map_err(LocalBackendErrorKind::FromTryIntError)
                        .map_err(|_err| todo!("Error transition"))?,
                ))
            })
            .filter_map(RusticResult::ok);

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
    /// * [`LocalBackendErrorKind::ReadingContentsOfFileFailed`] - If the file could not be read.
    ///
    /// [`LocalBackendErrorKind::ReadingContentsOfFileFailed`]: LocalBackendErrorKind::ReadingContentsOfFileFailed
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
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
    /// * [`LocalBackendErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalBackendErrorKind::CouldNotSeekToPositionInFile`] - If the file could not be sought to the given position.
    /// * [`LocalBackendErrorKind::FromTryIntError`] - If the length of the file could not be converted to u32.
    /// * [`LocalBackendErrorKind::ReadingExactLengthOfFileFailed`] - If the length of the file could not be read.
    ///
    /// [`LocalBackendErrorKind::OpeningFileFailed`]: LocalBackendErrorKind::OpeningFileFailed
    /// [`LocalBackendErrorKind::CouldNotSeekToPositionInFile`]: LocalBackendErrorKind::CouldNotSeekToPositionInFile
    /// [`LocalBackendErrorKind::FromTryIntError`]: LocalBackendErrorKind::FromTryIntError
    /// [`LocalBackendErrorKind::ReadingExactLengthOfFileFailed`]: LocalBackendErrorKind::ReadingExactLengthOfFileFailed
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let mut file =
            File::open(self.path(tpe, id)).map_err(LocalBackendErrorKind::OpeningFileFailed)?;
        _ = file
            .seek(SeekFrom::Start(offset.into()))
            .map_err(LocalBackendErrorKind::CouldNotSeekToPositionInFile)?;
        let mut vec = vec![
            0;
            length
                .try_into()
                .map_err(LocalBackendErrorKind::FromTryIntError)?
        ];
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
    /// * [`LocalBackendErrorKind::DirectoryCreationFailed`] - If the directory could not be created.
    ///
    /// [`LocalBackendErrorKind::DirectoryCreationFailed`]: LocalBackendErrorKind::DirectoryCreationFailed
    fn create(&self) -> RusticResult<()> {
        trace!("creating repo at {:?}", self.path);
        fs::create_dir_all(&self.path).map_err(LocalBackendErrorKind::DirectoryCreationFailed)?;

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
    /// * [`LocalBackendErrorKind::OpeningFileFailed`] - If the file could not be opened.
    /// * [`LocalBackendErrorKind::FromTryIntError`] - If the length of the bytes could not be converted to u64.
    /// * [`LocalBackendErrorKind::SettingFileLengthFailed`] - If the length of the file could not be set.
    /// * [`LocalBackendErrorKind::CouldNotWriteToBuffer`] - If the bytes could not be written to the file.
    /// * [`LocalBackendErrorKind::SyncingOfOsMetadataFailed`] - If the metadata of the file could not be synced.
    ///
    /// [`LocalBackendErrorKind::OpeningFileFailed`]: LocalBackendErrorKind::OpeningFileFailed
    /// [`LocalBackendErrorKind::FromTryIntError`]: LocalBackendErrorKind::FromTryIntError
    /// [`LocalBackendErrorKind::SettingFileLengthFailed`]: LocalBackendErrorKind::SettingFileLengthFailed
    /// [`LocalBackendErrorKind::CouldNotWriteToBuffer`]: LocalBackendErrorKind::CouldNotWriteToBuffer
    /// [`LocalBackendErrorKind::SyncingOfOsMetadataFailed`]: LocalBackendErrorKind::SyncingOfOsMetadataFailed
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        buf: Bytes,
    ) -> RusticResult<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
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
    /// * [`LocalBackendErrorKind::FileRemovalFailed`] - If the file could not be removed.
    ///
    /// [`LocalBackendErrorKind::FileRemovalFailed`]: LocalBackendErrorKind::FileRemovalFailed
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
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
