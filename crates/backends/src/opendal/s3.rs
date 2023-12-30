use std::collections::HashMap;

use anyhow::Result;
use itertools::Itertools;
use url::{self, Url};

use crate::opendal::OpenDALBackend;

#[derive(Clone, Debug)]
pub struct S3Backend(OpenDALBackend);

impl rustic_core::ReadBackend for S3Backend {
    fn location(&self) -> String {
        <OpenDALBackend as rustic_core::ReadBackend>::location(&self.0)
    }

    fn list(&self, tpe: rustic_core::FileType) -> Result<Vec<rustic_core::Id>> {
        <OpenDALBackend as rustic_core::ReadBackend>::list(&self.0, tpe)
    }

    fn list_with_size(&self, tpe: rustic_core::FileType) -> Result<Vec<(rustic_core::Id, u32)>> {
        <OpenDALBackend as rustic_core::ReadBackend>::list_with_size(&self.0, tpe)
    }

    fn read_full(&self, tpe: rustic_core::FileType, id: &rustic_core::Id) -> Result<bytes::Bytes> {
        <OpenDALBackend as rustic_core::ReadBackend>::read_full(&self.0, tpe, id)
    }

    fn read_partial(
        &self,
        tpe: rustic_core::FileType,
        id: &rustic_core::Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<bytes::Bytes> {
        <OpenDALBackend as rustic_core::ReadBackend>::read_partial(
            &self.0, tpe, id, _cacheable, offset, length,
        )
    }
}

impl rustic_core::WriteBackend for S3Backend {
    fn create(&self) -> Result<()> {
        <OpenDALBackend as rustic_core::WriteBackend>::create(&self.0)
    }

    fn write_bytes(
        &self,
        tpe: rustic_core::FileType,
        id: &rustic_core::Id,
        _cacheable: bool,
        buf: bytes::Bytes,
    ) -> Result<()> {
        <OpenDALBackend as rustic_core::WriteBackend>::write_bytes(
            &self.0, tpe, id, _cacheable, buf,
        )
    }

    fn remove(
        &self,
        tpe: rustic_core::FileType,
        id: &rustic_core::Id,
        _cacheable: bool,
    ) -> Result<()> {
        <OpenDALBackend as rustic_core::WriteBackend>::remove(&self.0, tpe, id, _cacheable)
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
    pub fn new(path: &str, mut options: HashMap<String, String>) -> Result<Self> {
        let mut url = Url::parse(path)?;
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
}
