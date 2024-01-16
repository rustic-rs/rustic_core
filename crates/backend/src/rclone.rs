use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

use anyhow::Result;
use bytes::Bytes;
use log::{debug, info, warn};
use rand::{
    distributions::{Alphanumeric, DistString},
    thread_rng,
};

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
}

impl Drop for RcloneBackend {
    /// Kill the child process.
    fn drop(&mut self) {
        debug!("killing rclone.");
        self.child.kill().unwrap();
    }
}

/// Get the rclone version.
///
/// # Errors
///
/// * [`RcloneErrorKind::FromIoError`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::FromUtf8Error`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::NoOutputForRcloneVersion`] - If the rclone version could not be determined.
/// * [`RcloneErrorKind::FromParseIntError`] - If the rclone version could not be determined.
///
/// # Returns
///
/// The rclone version as a tuple of (major, minor, patch).
///
/// [`RcloneErrorKind::FromIoError`]: RcloneErrorKind::FromIoError
/// [`RcloneErrorKind::FromUtf8Error`]: RcloneErrorKind::FromUtf8Error
/// [`RcloneErrorKind::NoOutputForRcloneVersion`]: RcloneErrorKind::NoOutputForRcloneVersion
/// [`RcloneErrorKind::FromParseIntError`]: RcloneErrorKind::FromParseIntError
fn rclone_version() -> Result<(i32, i32, i32)> {
    let rclone_version_output = Command::new("rclone")
        .arg("version")
        .output()
        .map_err(RcloneErrorKind::FromIoError)?
        .stdout;
    let rclone_version = std::str::from_utf8(&rclone_version_output)
        .map_err(RcloneErrorKind::FromUtf8Error)?
        .lines()
        .next()
        .ok_or_else(|| RcloneErrorKind::NoOutputForRcloneVersion)?
        .trim_start_matches(|c: char| !c.is_numeric());

    let versions: Vec<&str> = rclone_version.split(&['.', '-', ' '][..]).collect();
    let major = versions[0]
        .parse::<i32>()
        .map_err(RcloneErrorKind::FromParseIntError)?;
    let minor = versions[1]
        .parse::<i32>()
        .map_err(RcloneErrorKind::FromParseIntError)?;
    let patch = versions[2]
        .parse::<i32>()
        .map_err(RcloneErrorKind::FromParseIntError)?;
    Ok((major, minor, patch))
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
    pub fn new(
        url: impl AsRef<str>,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        match rclone_version() {
            Ok((major, minor, patch)) => {
                if major
                    .cmp(&1)
                    .then(minor.cmp(&52))
                    .then(patch.cmp(&2))
                    .is_lt()
                {
                    // TODO: This should be an error, and explicitly agreed to with a flag passed to `rustic`,
                    // check #812 for details
                    // for rclone < 1.52.2 setting user/password via env variable doesn't work. This means
                    // we are setting up an rclone without authentication which is a security issue!
                    // (however, it still works, so we give a warning)
                    warn!(
                "Using rclone without authentication! Upgrade to rclone >= 1.52.2 (current version: {major}.{minor}.{patch})!"
            );
                }
            }
            Err(err) => warn!("Could not determine rclone version: {err}"),
        }

        let user = Alphanumeric.sample_string(&mut thread_rng(), 12);
        let password = Alphanumeric.sample_string(&mut thread_rng(), 12);

        let args = ["serve", "restic", url.as_ref(), "--addr", "localhost:0"];
        debug!("starting rclone with args {args:?}");

        let mut child = Command::new("rclone")
            .env("RCLONE_USER", &user)
            .env("RCLONE_PASS", &password)
            .args(args)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RcloneErrorKind::FromIoError)?;

        let mut stderr = BufReader::new(
            child
                .stderr
                .take()
                .ok_or_else(|| RcloneErrorKind::NoStdOutForRclone)?,
        );
        let rest_url = loop {
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
        };

        let _join_handle = std::thread::spawn(move || loop {
            let mut line = String::new();
            if stderr.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if !line.is_empty() {
                info!("rclone output: {line}");
            }
        });

        if !rest_url.starts_with("http://") {
            return Err(RcloneErrorKind::UrlNotStartingWithHttp(rest_url).into());
        }

        let rest_url =
            "http://".to_string() + user.as_str() + ":" + password.as_str() + "@" + &rest_url[7..];

        debug!("using REST backend with url {}.", url.as_ref());
        let rest = RestBackend::new(rest_url, options)?;
        Ok(Self {
            child,
            url: String::from(url.as_ref()),
            rest,
        })
    }
}

impl ReadBackend for RcloneBackend {
    /// Returns the location of the backend.
    fn location(&self) -> String {
        let mut location = "rclone:".to_string();
        location.push_str(&self.url);
        location
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
