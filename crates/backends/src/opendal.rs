use std::{collections::HashMap, path::PathBuf, str::FromStr};

use anyhow::Result;
use bytes::Bytes;
#[allow(unused_imports)]
use cached::proc_macro::cached;
use itertools::Itertools;
use log::trace;
use once_cell::sync::Lazy;
use opendal::{
    layers::{BlockingLayer, LoggingLayer, RetryLayer},
    BlockingOperator, ErrorKind, Metakey, Operator, Scheme,
};
use rayon::prelude::*;
use url::{self, Url};

use rustic_core::{
    backend::{FileType, ReadBackend, WriteBackend, ALL_FILE_TYPES},
    id::Id,
};

mod consts {
    /// Default number of retries
    pub(super) const DEFAULT_RETRY: usize = 5;
}

#[derive(Clone, Debug)]
pub struct OpenDALBackend {
    operator: BlockingOperator,
}

static RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});

impl OpenDALBackend {
    /// Convenience method to directly create a new s3 backend
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the s3 bucket
    /// * `options` - Additional options for the s3 backend
    ///
    /// # Notes
    ///
    /// The path should be something like "`https://s3.amazonaws.com/bucket/my/repopath`"
    pub fn new_s3(path: &str, mut options: HashMap<String, String>) -> Result<Self> {
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

        Self::new("s3", options)
    }

    pub fn new(path: &str, options: HashMap<String, String>) -> Result<Self> {
        let max_retries = match options.get("retry").map(std::string::String::as_str) {
            Some("false" | "off") => 0,
            None | Some("default") => consts::DEFAULT_RETRY,
            Some(value) => usize::from_str(value)?,
        };

        let schema = Scheme::from_str(path)?;
        let _guard = RUNTIME.enter();
        let operator = Operator::via_map(schema, options)?
            .layer(RetryLayer::new().with_max_times(max_retries).with_jitter())
            .layer(LoggingLayer::default())
            .layer(BlockingLayer::create()?)
            .blocking();
        Ok(Self { operator })
    }

    fn path(&self, tpe: FileType, id: &Id) -> String {
        let hex_id = id.to_hex();
        match tpe {
            FileType::Config => PathBuf::from("config"),
            FileType::Pack => PathBuf::from("data").join(&hex_id[0..2]).join(hex_id),
            _ => PathBuf::from(tpe.dirname()).join(hex_id),
        }
        .to_string_lossy()
        .to_string()
    }
}

impl ReadBackend for OpenDALBackend {
    /// Returns the location of the backend.
    ///
    /// This is `local:<path>`.
    fn location(&self) -> String {
        let mut location = "opendal:".to_string();
        location.push_str(self.operator.info().name());
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
    fn list(&self, tpe: FileType) -> Result<Vec<Id>> {
        trace!("listing tpe: {tpe:?}");
        if tpe == FileType::Config {
            return Ok(if self.operator.is_exist("config")? {
                vec![Id::default()]
            } else {
                Vec::new()
            });
        }

        Ok(self
            .operator
            .list_with(&(tpe.dirname().to_string() + "/"))
            .recursive(true)
            .call()?
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .map(|e| Id::from_hex(e.name()))
            .filter_map(Result::ok)
            .collect())
    }

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        trace!("listing tpe: {tpe:?}");
        if tpe == FileType::Config {
            return match self.operator.stat("config") {
                Ok(entry) => Ok(vec![(Id::default(), entry.content_length().try_into()?)]),
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(Vec::new()),
                Err(err) => Err(err.into()),
            };
        }

        Ok(self
            .operator
            .list_with(&(tpe.dirname().to_string() + "/"))
            .recursive(true)
            .metakey(Metakey::ContentLength)
            .call()?
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .map(|e| -> Result<(Id, u32)> {
                Ok((
                    Id::from_hex(e.name())?,
                    e.metadata().content_length().try_into()?,
                ))
            })
            .filter_map(Result::ok)
            .collect())
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");

        Ok(self.operator.read(&self.path(tpe, id))?.into())
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let range = u64::from(offset)..u64::from(offset + length);
        Ok(self
            .operator
            .read_with(&self.path(tpe, id))
            .range(range)
            .call()?
            .into())
    }
}

impl WriteBackend for OpenDALBackend {
    /// Create a repository on the backend.
    fn create(&self) -> Result<()> {
        trace!("creating repo at {:?}", self.location());

        for tpe in ALL_FILE_TYPES {
            self.operator
                .create_dir(&(tpe.dirname().to_string() + "/"))?;
        }
        // creating 256 dirs can be slow on remote backends, hence we parallelize it.
        (0u8..=255).into_par_iter().try_for_each(|i| {
            self.operator.create_dir(
                &(PathBuf::from("data")
                    .join(hex::encode([i]))
                    .to_string_lossy()
                    .to_string()
                    + "/"),
            )
        })?;

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
    fn write_bytes(&self, tpe: FileType, id: &Id, _cacheable: bool, buf: Bytes) -> Result<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        self.operator.write(&filename, buf)?;
        Ok(())
    }

    /// Remove the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> Result<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        self.operator.delete(&filename)?;
        Ok(())
    }
}
