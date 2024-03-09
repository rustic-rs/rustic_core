#[cfg(feature = "s3")]
pub mod s3;
#[cfg(all(unix, feature = "sftp"))]
pub mod sftp;

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::OnceLock};

use anyhow::Result;
use bytes::Bytes;
use log::trace;
use opendal::{
    layers::{BlockingLayer, ConcurrentLimitLayer, LoggingLayer, RetryLayer},
    BlockingOperator, ErrorKind, Metakey, Operator, Scheme,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tokio::runtime::Runtime;

use rustic_core::{FileType, Id, ReadBackend, WriteBackend, ALL_FILE_TYPES};

mod consts {
    /// Default number of retries
    pub(super) const DEFAULT_RETRY: usize = 5;
}

#[derive(Clone, Debug)]
pub struct OpenDALBackend {
    operator: BlockingOperator,
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    })
}

impl OpenDALBackend {
    /// Create a new openDAL backend.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the OpenDAL backend.
    /// * `options` - Additional options for the OpenDAL backend.
    pub fn new(path: impl AsRef<str>, options: HashMap<String, String>) -> Result<Self> {
        let max_retries = match options.get("retry").map(String::as_str) {
            Some("false" | "off") => 0,
            None | Some("default") => consts::DEFAULT_RETRY,
            Some(value) => usize::from_str(value)?,
        };
        let connections = options
            .get("connections")
            .map(|c| usize::from_str(c))
            .transpose()?;

        let schema = Scheme::from_str(path.as_ref())?;
        let mut operator = Operator::via_map(schema, options)?
            .layer(RetryLayer::new().with_max_times(max_retries).with_jitter());

        if let Some(connections) = connections {
            operator = operator.layer(ConcurrentLimitLayer::new(connections));
        }

        let _guard = runtime().enter();
        let operator = operator
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
