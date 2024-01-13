use std::collections::HashMap;

use anyhow::Result;
use itertools::Itertools;
use url::{self, Url};

use crate::opendal::OpenDALBackend;
use bytes::Bytes;
use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

#[derive(Clone, Debug)]
pub struct S3Backend(OpenDALBackend);

impl ReadBackend for S3Backend {
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

impl WriteBackend for S3Backend {
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

impl S3Backend {
    /// Create a new S3 backend.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the s3 bucket
    /// * `options` - Additional options for the s3 backend
    ///
    /// # Notes
    ///
    /// The path should be something like "`https://s3.amazonaws.com/bucket/my/repopath`"
    pub fn new(path: impl AsRef<str>, mut options: HashMap<String, String>) -> Result<Self> {
        let mut url = Url::parse(path.as_ref())?;
        if let Some(mut path_segments) = url.path_segments() {
            if let Some(bucket) = path_segments.next() {
                let _ = options.insert("bucket".to_string(), bucket.to_string());
            }
            let root = path_segments.join("/");
            if !root.is_empty() {
                let _ = options.insert("root".to_string(), root);
            }
        }
        if url.has_host() {
            if url.scheme().is_empty() {
                url.set_scheme("https")
                    .expect("could not set scheme to https");
            }
            url.set_path("");
            url.set_query(None);
            url.set_fragment(None);
            let _ = options.insert("endpoint".to_string(), url.to_string());
        }
        _ = options
            .entry("region".to_string())
            .or_insert_with(|| "auto".to_string());

        Ok(Self(OpenDALBackend::new("s3", options)?))
    }

    pub fn to_inner(self) -> OpenDALBackend {
        self.0
    }
}
