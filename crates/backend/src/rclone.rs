use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    thread::JoinHandle,
};

use anyhow::Result;
use bytes::Bytes;
use log::{debug, info};
use rand::{
    distributions::{Alphanumeric, DistString},
    thread_rng,
};

use semver::{BuildMetadata, Prerelease, Version, VersionReq};
use shell_words::split;

use crate::{error::RcloneErrorKind, rest::RestBackend};

use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

pub(super) mod constants {
    /// The string to search for in the rclone output.
    pub(super) const SEARCHSTRING: &str = "Serving restic REST API on ";
}

/// `RcloneBackend` is a backend that uses rclone to access a remote backend.
#[derive(Debug)]
pub struct RcloneBackend {
    /// The REST backend.
    rest: RestBackend,
    /// The url of the backend.
    url: String,
    /// The child data contains the child process and is used to kill the child process when the backend is dropped.
    child: Child,
    /// The [`JoinHandle`] of the thread printing rclone's output
    handle: Option<JoinHandle<()>>,
}

impl Drop for RcloneBackend {
    /// Kill the child process.
    fn drop(&mut self) {
        debug!("killing rclone.");
        self.child.kill().unwrap();
        self.handle.take().map(JoinHandle::join);
    }
}

/// Check the rclone version.
///
/// # Arguments
///
/// * `rclone_version_output` - The output of `rclone version`.
///
/// # Errors
///
/// * [`RcloneErrorKind::FromIoError`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::FromUtf8Error`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::NoOutputForRcloneVersion`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::FromParseVersion`] - If the rclone version could not be determined.
///
/// # Returns
///
/// * `Ok(())` - If the rclone version is supported.
///
/// [`RcloneErrorKind::FromIoError`]: RcloneErrorKind::FromIoError
/// [`RcloneErrorKind::FromUtf8Error`]: RcloneErrorKind::FromUtf8Error
/// [`RcloneErrorKind::NoOutputForRcloneVersion`]: RcloneErrorKind::NoOutputForRcloneVersion
/// [`RcloneErrorKind::FromParseVersion`]: RcloneErrorKind::FromParseVersion
fn check_clone_version(rclone_version_output: &[u8]) -> Result<()> {
    let rclone_version = std::str::from_utf8(rclone_version_output)
        .map_err(RcloneErrorKind::FromUtf8Error)?
        .lines()
        .next()
        .ok_or_else(|| RcloneErrorKind::NoOutputForRcloneVersion)?
        .trim_start_matches(|c: char| !c.is_numeric());

    // for rclone < 1.52.2 setting user/password via env variable doesn't work. This means
    // we are setting up an rclone without authentication which is a security issue!
    let mut parsed_version = Version::parse(rclone_version)?;
    parsed_version.pre = Prerelease::EMPTY;
    parsed_version.build = BuildMetadata::EMPTY;

    if VersionReq::parse("<1.52.2")?.matches(&parsed_version) {
        return Err(
            RcloneErrorKind::RCloneWithoutAuthentication(rclone_version.to_string()).into(),
        );
    }

    Ok(())
}

impl RcloneBackend {
    /// Create a new [`RcloneBackend`] from a given url.
    ///
    /// # Arguments
    ///
    /// * `url` - The url to create the [`RcloneBackend`] from.
    ///
    /// # Errors
    ///
    /// * [`RcloneErrorKind::FromIoError`] - If the rclone version could not be determined.
    /// * [`RcloneErrorKind::NoStdOutForRclone`] - If the rclone version could not be determined.
    /// * [`RcloneErrorKind::RCloneExitWithBadStatus`] - If rclone exited with a bad status.
    /// * [`RcloneErrorKind::UrlNotStartingWithHttp`] - If the URL does not start with `http`.
    ///
    /// [`RcloneErrorKind::FromIoError`]: RcloneErrorKind::FromIoError
    /// [`RcloneErrorKind::NoStdOutForRclone`]: RcloneErrorKind::NoStdOutForRclone
    /// [`RcloneErrorKind::RCloneExitWithBadStatus`]: RcloneErrorKind::RCloneExitWithBadStatus
    /// [`RcloneErrorKind::UrlNotStartingWithHttp`]: RcloneErrorKind::UrlNotStartingWithHttp
    pub fn new(url: impl AsRef<str>, options: HashMap<String, String>) -> Result<Self> {
        let rclone_command = options.get("rclone-command");
        let use_password = options
            .get("use-password")
            .map(|v| v.parse())
            .transpose()?
            .unwrap_or(true);

        if use_password && rclone_command.is_none() {
            let rclone_version_output = Command::new("rclone")
                .arg("version")
                .output()
                .map_err(RcloneErrorKind::FromIoError)?
                .stdout;

            // if we want to use a password and rclone_command is not explicitly set, we check for a rclone version supporting
            // user/password via env variables
            check_clone_version(rclone_version_output.as_slice())?;
        }

        let user = Alphanumeric.sample_string(&mut thread_rng(), 12);
        let password = Alphanumeric.sample_string(&mut thread_rng(), 12);

        let mut rclone_command = split(
            rclone_command
                .map(String::as_str)
                .unwrap_or("rclone serve restic --addr localhost:0"),
        )?;
        rclone_command.push(url.as_ref().to_string());
        debug!("starting rclone via {rclone_command:?}");

        let mut command = Command::new(&rclone_command[0]);
        if use_password {
            command
                .env("RCLONE_USER", &user)
                .env("RCLONE_PASS", &password);
        }
        let mut child = command
            .args(&rclone_command[1..])
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RcloneErrorKind::FromIoError)?;

        let mut stderr = BufReader::new(
            child
                .stderr
                .take()
                .ok_or_else(|| RcloneErrorKind::NoStdOutForRclone)?,
        );

        let mut rest_url = match options.get("rest-url") {
            None => {
                loop {
                    if let Some(status) = child.try_wait().map_err(RcloneErrorKind::FromIoError)? {
                        return Err(RcloneErrorKind::RCloneExitWithBadStatus(status).into());
                    }
                    let mut line = String::new();
                    _ = stderr
                        .read_line(&mut line)
                        .map_err(RcloneErrorKind::FromIoError)?;
                    match line.find(constants::SEARCHSTRING) {
                        Some(result) => {
                            if let Some(url) = line.get(result + constants::SEARCHSTRING.len()..) {
                                // rclone > 1.61 adds brackets around the url, so remove those
                                let brackets: &[_] = &['[', ']'];
                                break url.trim_end().trim_matches(brackets).to_string();
                            }
                        }
                        None if !line.is_empty() => info!("rclone output: {line}"),
                        _ => {}
                    }
                }
            }
            Some(url) => url.to_string(),
        };

        if use_password {
            if !rest_url.starts_with("http://") {
                return Err(RcloneErrorKind::UrlNotStartingWithHttp(rest_url).into());
            }
            rest_url = format!("http://{user}:{password}@{}", &rest_url[7..]);
        }

        debug!("using REST backend with url {}.", url.as_ref());
        let rest = RestBackend::new(rest_url, options)?;

        let handle = Some(std::thread::spawn(move || loop {
            let mut line = String::new();
            if stderr.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if !line.is_empty() {
                info!("rclone output: {line}");
            }
        }));

        Ok(Self {
            child,
            url: String::from(url.as_ref()),
            rest,
            handle,
        })
    }
}

impl ReadBackend for RcloneBackend {
    /// Returns the location of the backend.
    fn location(&self) -> String {
        "rclone:".to_string() + &self.url
    }

    /// Returns the size of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    ///
    /// If the size could not be determined.
    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.rest.list_with_size(tpe)
    }

    /// Reads full data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Returns
    ///
    /// The data read.
    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.rest.read_full(tpe, id)
    }

    /// Reads partial data of the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the data should be cached.
    /// * `offset` - The offset to read from.
    /// * `length` - The length to read.
    ///
    /// # Returns
    ///
    /// The data read.
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        self.rest.read_partial(tpe, id, cacheable, offset, length)
    }
}

impl WriteBackend for RcloneBackend {
    /// Creates a new file.
    fn create(&self) -> Result<()> {
        self.rest.create()
    }

    /// Writes bytes to the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the data should be cached.
    /// * `buf` - The data to write.
    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        self.rest.write_bytes(tpe, id, cacheable, buf)
    }

    /// Removes the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        self.rest.remove(tpe, id, cacheable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    #[rstest]
    #[case(b"rclone v1.52.2\n- os/arch: linux/amd64\n- go version: go1.14.4\n")]
    #[case(b"rclone v1.66.0\n- os/version: Microsoft Windows 11 Pro 23H2 (64 bit)\n- os/kernel: 10.0.22631.3155 (x86_64)\n- os/type: windows\n- os/arch: amd64\n- go/version: go1.22.1\n- go/linking: static\n- go/tags: cmount")]
    #[case(b"rclone v1.63.0-beta.7022.e649cf4d5\n- os/arch: linux/amd64\n- go version: go1.14.4\n")]
    fn test_check_clone_version_passes(#[case] rclone_version_output: &[u8]) {
        assert!(check_clone_version(rclone_version_output).is_ok());
    }

    #[rstest]
    #[case(b"")]
    #[case(b"rclone v1.52.1\n- os/arch: linux/amd64\n- go version: go1.14.4\n")]
    #[case(b"rclone v1.51.3-beta\n- os/arch: linux/amd64\n- go version: go1.14.4\n")]
    fn test_check_clone_version_fails(#[case] rclone_version_output: &[u8]) {
        assert!(check_clone_version(rclone_version_output).is_err());
    }
}
