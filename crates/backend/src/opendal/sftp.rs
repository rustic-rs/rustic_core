use std::collections::HashMap;

use anyhow::Result;
use url::{self, Url};

use crate::opendal::OpenDALBackend;
use bytes::Bytes;
use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

/// SFTP backend for Rustic. A wrapper around the `OpenDAL` backend.
#[derive(Clone, Debug)]
pub struct SftpBackend(OpenDALBackend);

impl ReadBackend for SftpBackend {
    fn location(&self) -> String {
        self.0.location()
    }

    fn list(&self, tpe: FileType) -> Result<Vec<Id>> {
        self.0.list(tpe)
    }

    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.0.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.0.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        self.0.read_partial(tpe, id, cacheable, offset, length)
    }
}

impl WriteBackend for SftpBackend {
    fn create(&self) -> Result<()> {
        self.0.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        self.0.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        self.0.remove(tpe, id, cacheable)
    }
}

impl SftpBackend {
    /// Create a new SFTP backend.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the sftp server
    /// * `options` - Additional options for the SFTP backend
    ///
    /// # Errors
    ///
    /// If the path is not a valid url, an error is returned.
    ///
    /// # Returns
    ///
    /// The new SFTP backend.
    ///
    /// # Notes
    ///
    /// The path should be something like "`sftp://user@host:port/path`"
    pub fn new(path: impl AsRef<str>, mut options: HashMap<String, String>) -> Result<Self> {
        let url = Url::parse(&("sftp://".to_string() + path.as_ref()))?;

        let user = url.username();
        if !user.is_empty() {
            _ = options
                .entry("user".to_string())
                .or_insert_with(|| user.to_string());
        }
        if let Some(host) = url.host() {
            _ = options
                .entry("endpoint".to_string())
                .or_insert_with(|| host.to_string());
        }
        _ = options
            .entry("root".to_string())
            .or_insert_with(|| url.path().to_string());

        Ok(Self(OpenDALBackend::new("sftp", options)?))
    }

    pub fn to_inner(self) -> OpenDALBackend {
        self.0
    }
}
