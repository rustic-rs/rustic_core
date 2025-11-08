use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
};

use aho_corasick::AhoCorasick;
use bytes::Bytes;
use log::{debug, error, trace, warn};
use walkdir::WalkDir;

use rustic_core::{
    ALL_FILE_TYPES, CommandInput, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult,
    WriteBackend,
};

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
    /// * `options` - Additional options for the backend
    ///
    /// # Errors
    ///
    /// * If the directory could not be created.
    ///
    /// # Options
    ///
    /// * `post-create-command` - The command to call after a file was created.
    /// * `post-delete-command` - The command to call after a file was deleted.
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

    /// Base path of the given file type and id.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Returns
    ///
    /// The base path of the file.
    fn base_path(&self, tpe: FileType, id: &Id) -> PathBuf {
        let hex_id = id.to_hex();
        match tpe {
            FileType::Config => self.path.clone(),
            FileType::Pack => self.path.join("data").join(&hex_id[0..2]),
            _ => self.path.join(tpe.dirname()),
        }
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
            _ => self.base_path(tpe, id).join(hex_id),
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
    /// * If the patterns could not be compiled.
    /// * If the command could not be parsed.
    /// * If the command could not be executed.
    /// * If the command was not successful.
    ///
    /// # Notes
    ///
    /// The following placeholders are supported:
    /// * `%file` - The path to the file.
    /// * `%type` - The type of the file.
    /// * `%id` - The id of the file.
    fn call_command(tpe: FileType, id: &Id, filename: &Path, command: &str) -> RusticResult<()> {
        let id = id.to_hex();

        let patterns = &["%file", "%type", "%id"];

        let ac = AhoCorasick::new(patterns).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Experienced an error building AhoCorasick automaton for command replacement.",
                err,
            )
            .ask_report()
        })?;

        let replace_with = &[filename.to_str().unwrap(), tpe.dirname(), id.as_str()];

        let actual_command = ac.replace_all(command, replace_with);

        debug!("calling {actual_command}...");

        let command: CommandInput = actual_command.parse().map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to parse command input: `{command}` is not a valid command.",
                err,
            )
            .attach_context("command", actual_command)
            .attach_context("replacement", replace_with.join(", "))
            .ask_report()
        })?;

        let status = Command::new(command.command())
            .args(command.args())
            .status()
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::ExternalCommand,
                    "Failed to execute `{command}`. Please check the command and try again.",
                    err,
                )
                .attach_context("command", command.to_string())
            })?;

        if !status.success() {
            return Err(RusticError::new(
                ErrorKind::ExternalCommand,
                "Command was not successful: `{command}` failed with status `{status}`.",
            )
            .attach_context("command", command.to_string())
            .attach_context("file_name", replace_with[0])
            .attach_context("file_type", replace_with[1])
            .attach_context("id", replace_with[2])
            .attach_context("status", status.to_string()));
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
            // TODO: What to do with errors?
            .inspect(|r| {
                if let Err(err) = r {
                    error!("Error while listing files: {err:?}");
                }
            })
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
    /// * If the metadata of the file could not be queried.
    /// * If the length of the file could not be converted to u32.
    /// * If the metadata of the file could not be queried.
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        trace!("listing tpe: {tpe:?}");
        let path = self.path.join(tpe.dirname());

        if tpe == FileType::Config {
            return Ok(if path.exists() {
                vec![(Id::default(), {
                    let metadata = path.metadata().map_err(|err|
                            RusticError::with_source(
                                ErrorKind::Backend,
                                "Failed to query metadata of the file `{path}`. Please check the file and try again.",
                                err
                            )
                            .attach_context("path", path.to_string_lossy())
                        )?;

                    metadata.len().try_into().map_err(|err| {
                        RusticError::with_source(
                            ErrorKind::Backend,
                            "Failed to convert file length `{length}` to u32.",
                            err,
                        )
                        .attach_context("length", metadata.len().to_string())
                        .ask_report()
                    })?
                })]
            } else {
                Vec::new()
            });
        }

        let walker = WalkDir::new(path)
            .into_iter()
            .inspect(|r| {
                if let Err(err) = r {
                    error!("Error while listing files: {err:?}");
                }
            })
            .filter_map(walkdir::Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| -> RusticResult<_> {
                Ok((
                    e.file_name().to_string_lossy().parse()?,
                    {
                        let metadata = e.metadata()
                        .map_err(|err|
                            RusticError::with_source(
                                ErrorKind::Backend,
                                "Failed to query metadata of the file `{path}`. Please check the file and try again.",
                                err
                            )
                            .attach_context("path", e.path().to_string_lossy())
                        )
                        ?;

                        metadata
                        .len()
                        .try_into()
                        .map_err(|err|
                            RusticError::with_source(
                                ErrorKind::Backend,
                                "Failed to convert file length `{length}` to u32.",
                                err
                            )
                            .attach_context("length", metadata.len().to_string())
                            .ask_report()
                        )?
                    },
                ))
            })
            .inspect(|r| {
                if let Err(err) = r {
                    error!("Error while listing files: {}", err.display_log());
                }
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
    /// * If the file could not be read.
    /// * If the file could not be found.
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");
        Ok(fs::read(self.path(tpe, id))
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Failed to read the contents of the file. Please check the file and try again.",
                    err,
                )
                .attach_context("path", self.path(tpe, id).to_string_lossy())
            })?
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
    /// * If the file could not be opened.
    /// * If the file could not be sought to the given position.
    /// * If the length of the file could not be converted to u32.
    /// * If the exact length of the file could not be read.
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let filename = self.path(tpe, id);
        let mut file = File::open(filename.clone()).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Failed to open the file `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", filename.to_string_lossy())
        })?;
        _ = file.seek(SeekFrom::Start(offset.into())).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Failed to seek to the position `{offset}` in the file `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", self.path(tpe, id).to_string_lossy())
            .attach_context("offset", offset.to_string())
        })?;

        let mut vec = vec![
            0;
            length.try_into().map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Failed to convert length `{length}` to u64.",
                    err,
                )
                .attach_context("length", length.to_string())
                .ask_report()
            })?
        ];

        file.read_exact(&mut vec).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Failed to read the exact length `{length}` of the file `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", self.path(tpe, id).to_string_lossy())
            .attach_context("length", length.to_string())
        })?;

        Ok(vec.into())
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        // For local backends, we can provide the filesystem path as the warmup path
        // though warmup is not typically needed for local storage
        self.path(tpe, id).to_string_lossy().to_string()
    }
}

impl WriteBackend for LocalBackend {
    /// Create a repository on the backend.
    ///
    /// # Errors
    ///
    /// * If the directory could not be created.
    fn create(&self) -> RusticResult<()> {
        trace!("creating repo at {}", self.path.display());
        fs::create_dir_all(&self.path).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to create the directory `{path}`. Please check the path and try again.",
                err,
            )
            .attach_context("path", self.path.display().to_string())
        })?;

        for tpe in ALL_FILE_TYPES {
            let path = self.path.join(tpe.dirname());
            fs::create_dir_all(path.clone()).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::InputOutput,
                    "Failed to create the directory `{path}`. Please check the path and try again.",
                    err,
                )
                .attach_context("path", path.display().to_string())
            })?;
        }

        for i in 0u8..=255 {
            let path = self.path.join("data").join(hex::encode([i]));
            fs::create_dir_all(path.clone()).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::InputOutput,
                    "Failed to create the directory `{path}`. Please check the path and try again.",
                    err,
                )
                .attach_context("path", path.display().to_string())
            })?;
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
    /// * If the file could not be opened.
    /// * If the length of the bytes could not be converted to u64.
    /// * If the length of the file could not be set.
    /// * If the bytes could not be written to the file.
    /// * If the OS Metadata could not be synced to disk.
    /// * If the file does not have a parent directory.
    /// * If the parent directory could not be created.
    /// * If the file cannot be opened, due to missing permissions.
    /// * If the file cannot be written to, due to lack of space on the disk.
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        buf: Bytes,
    ) -> RusticResult<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);

        let parent = self.base_path(tpe, id);

        // create parent directory if it does not exist
        fs::create_dir_all(&parent).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to create directories `{path}`. Does the directory already exist? Please check the file and try again.",
                err,
            )
            .attach_context("path", parent.display().to_string())
            .ask_report()
        })?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&filename)
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::InputOutput,
                    "Failed to open the file `{path}`. Please check the file and try again.",
                    err,
                )
                .attach_context("path", filename.to_string_lossy())
            })?;

        file.set_len(buf.len().try_into().map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to convert length `{length}` to u64.",
                err,
            )
            .attach_context("length", buf.len().to_string())
            .ask_report()
        })?)
        .map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to set the length of the file `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", filename.to_string_lossy())
        })?;

        file.write_all(&buf).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to write to the buffer: `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", filename.to_string_lossy())
        })?;

        file.sync_all().map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to sync OS Metadata to disk: `{path}`. Please check the file and try again.",
                err,
            )
            .attach_context("path", filename.to_string_lossy())
        })?;

        if let Some(command) = &self.post_create_command {
            if let Err(err) = Self::call_command(tpe, id, &filename, command) {
                warn!("post-create: {}", err.display_log());
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
    /// * If the file could not be removed.
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        fs::remove_file(&filename).map_err(|err|
            RusticError::with_source(
                ErrorKind::Backend,
                "Failed to remove the file `{path}`. Was the file already removed or is it in use? Please check the file and remove it manually.",
                err
            )
            .attach_context("path", filename.to_string_lossy())
        )?;
        if let Some(command) = &self.post_delete_command {
            if let Err(err) = Self::call_command(tpe, id, &filename, command) {
                warn!("post-delete: {}", err.display_log());
            }
        }
        Ok(())
    }
}
