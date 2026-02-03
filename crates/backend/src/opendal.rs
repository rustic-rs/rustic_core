/// `OpenDAL` backend for rustic.
use std::{collections::BTreeMap, str::FromStr, sync::OnceLock};

use bytes::Bytes;
use bytesize::ByteSize;
use log::{error, trace};
use opendal::{
    blocking::Operator,
    layers::{ConcurrentLimitLayer, LoggingLayer, RetryLayer, ThrottleLayer},
    options::{ListOptions, ReadOptions},
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tokio::runtime::Runtime;
use typed_path::UnixPathBuf;

use rustic_core::{
    ALL_FILE_TYPES, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
};

mod constants {
    /// Default number of retries
    pub(super) const DEFAULT_RETRY: usize = 5;
}

/// `OpenDALBackend` contains a wrapper around an blocking operator of the `OpenDAL` library.
#[derive(Clone, Debug)]
pub struct OpenDALBackend {
    operator: Operator,
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

/// Throttling parameters
///
/// Note: Throttle implements [`FromStr`] to read it from something like "10kiB,10MB"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Throttle {
    bandwidth: u32,
    burst: u32,
}

impl FromStr for Throttle {
    type Err = Box<RusticError>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut values = s
            .split(',')
            .map(|s| {
                ByteSize::from_str(s.trim()).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Parsing ByteSize from throttle string `{string}` failed",
                        err,
                    )
                    .attach_context("string", s)
                })
            })
            .map(|b| -> RusticResult<u32> {
                let bytesize = b?.as_u64();
                bytesize.try_into().map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Converting ByteSize `{bytesize}` to u32 failed",
                        err,
                    )
                    .attach_context("bytesize", bytesize.to_string())
                })
            });

        let bandwidth = values
            .next()
            .transpose()?
            .ok_or_else(|| RusticError::new(ErrorKind::MissingInput, "No bandwidth given."))?;

        let burst = values
            .next()
            .transpose()?
            .ok_or_else(|| RusticError::new(ErrorKind::MissingInput, "No burst given."))?;

        Ok(Self { bandwidth, burst })
    }
}

impl OpenDALBackend {
    /// Create a new openDAL backend.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the `OpenDAL` backend.
    /// * `options` - Additional options for the `OpenDAL` backend.
    ///
    /// # Errors
    ///
    /// * If the path is not a valid `OpenDAL` path.
    ///
    /// # Returns
    ///
    /// A new `OpenDAL` backend.
    pub fn new(path: impl AsRef<str>, options: BTreeMap<String, String>) -> RusticResult<Self> {
        let max_retries = match options.get("retry").map(String::as_str) {
            Some("false" | "off") => 0,
            None | Some("default") => constants::DEFAULT_RETRY,
            Some(value) => usize::from_str(value).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::InvalidInput,
                    "Parsing retry value `{value}` failed, the value must be a valid integer.",
                    err,
                )
                .attach_context("value", value.to_string())
            })?,
        };
        let connections = options
            .get("connections")
            .map(|c| {
                usize::from_str(c).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Parsing connections value `{value}` failed, the value must be a valid integer.",
                        err,
                    )
                    .attach_context("value", c)
                })
            })
            .transpose()?;

        let throttle = options
            .get("throttle")
            .map(|t| Throttle::from_str(t))
            .transpose()?;

        let scheme = path
            .as_ref()
            .split(':')
            .next()
            .unwrap_or_else(|| path.as_ref());

        let mut operator = opendal::Operator::via_iter(scheme, options)
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Creating Operator from path `{path}` failed. Please check the given schema and options.",
                    err,
                )
                .attach_context("path", path.as_ref().to_string())
                .attach_context("schema", scheme.to_string())
            })?
            .layer(RetryLayer::new().with_max_times(max_retries).with_jitter());

        if let Some(Throttle { bandwidth, burst }) = throttle {
            operator = operator.layer(ThrottleLayer::new(bandwidth, burst));
        }

        if let Some(connections) = connections {
            operator = operator.layer(ConcurrentLimitLayer::new(connections));
        }

        let _guard = runtime().enter();
        let operator = Operator::new(operator.layer(LoggingLayer::default())).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Creating blocking Operator from path `{path}` failed.",
                err,
            )
            .attach_context("path", path.as_ref().to_string())
        })?;

        Ok(Self { operator })
    }

    /// Return a path for the given file type and id.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Returns
    ///
    /// The path for the given file type and id.
    // Let's keep this for now, as it's being used in the trait implementations.
    #[allow(clippy::unused_self)]
    fn path(&self, tpe: FileType, id: &Id) -> String {
        let hex_id = id.to_hex();
        match tpe {
            FileType::Config => UnixPathBuf::from("config"),
            FileType::Pack => UnixPathBuf::from("data")
                .join(&hex_id[0..2])
                .join(&hex_id[..]),
            _ => UnixPathBuf::from(tpe.dirname()).join(&hex_id[..]),
        }
        .to_string()
    }
}

impl ReadBackend for OpenDALBackend {
    /// Returns the location of the backend.
    ///
    /// This is `opendal:<scheme>:<name>` (e.g., `opendal:gdrive:` for Google Drive).
    fn location(&self) -> String {
        let info = self.operator.info();
        format!("opendal:{}:{}", info.scheme(), info.name())
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
            return Ok(
                if self.operator.exists("config").map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Backend,
                        "Path `config` does not exist.",
                        err,
                    )
                    .ask_report()
                })? {
                    vec![Id::default()]
                } else {
                    Vec::new()
                },
            );
        }

        let path = tpe.dirname().to_string() + "/";
        let list_options = ListOptions {
            recursive: true,
            ..Default::default()
        };

        Ok(self
            .operator
            .list_options(&path, list_options)
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Listing all files of `{path}` failed in the backend. Please check if the given path is correct.",
                    err,
                )
                .attach_context("path", path)
                .attach_context("type", tpe.to_string())
            })?
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .filter_map(|e| e.name().parse().ok())
            .collect())
    }

    /// Lists all files with their size of the given type.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        trace!("listing tpe: {tpe:?}");
        if tpe == FileType::Config {
            return match self.operator.stat("config") {
                Ok(entry) => Ok(vec![(
                    Id::default(),
                    entry.content_length().try_into().map_err(|err| {
                        RusticError::with_source(
                            ErrorKind::Internal,
                            "Parsing content length `{length}` failed",
                            err,
                        )
                        .attach_context("length", entry.content_length().to_string())
                    })?,
                )]),
                Err(err) if err.kind() == opendal::ErrorKind::NotFound => Ok(Vec::new()),
                Err(err) => Err(err).map_err(|err|
                    RusticError::with_source(
                        ErrorKind::Backend,
                        "Getting Metadata of type `{type}` failed in the backend. Please check if `{type}` exists.",
                        err,
                    )
                    .attach_context("type", tpe.to_string())
                ),
            };
        }

        let path = tpe.dirname().to_string() + "/";
        let list_options = ListOptions {
            recursive: true,
            ..Default::default()
        };
        Ok(self
            .operator.list_options(&path, list_options)
            .map_err(|err|
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Listing all files of `{type}` in directory `{path}` and their sizes failed in the backend. Please check if the given path is correct.",
                    err,
                )
                .attach_context("path", path)
                .attach_context("type", tpe.to_string())
            )?
            .into_iter()
            .filter(|e| e.metadata().is_file())
            .map(|e| -> RusticResult<(Id, u32)> {
                Ok((
                    e.name().parse()?,
                    e.metadata()
                        .content_length()
                        .try_into()
                        .map_err(|err|
                            RusticError::with_source(
                                ErrorKind::Internal,
                                "Parsing content length `{length}` failed",
                                err,
                            )
                            .attach_context("length", e.metadata().content_length().to_string())
                        )?,
                ))
            })
            .inspect(|r| {
                if let Err(err) = r {
                    error!("Error while listing files: {}", err.display_log());
                }
            })
            .filter_map(RusticResult::ok)
            .collect())
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");

        let path = self.path(tpe, id);
        Ok(self
            .operator
            .read(&path)
            .map_err(|err|
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Reading file `{path}` failed in the backend. Please check if the given path is correct.",
                    err,
                )
                .attach_context("path", path)
                .attach_context("type", tpe.to_string())
                .attach_context("id", id.to_string())
            )?
            .to_bytes())
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let range = u64::from(offset)..u64::from(offset + length);
        let path = self.path(tpe, id);
        let read_options = ReadOptions {
            range: range.into(),
            ..Default::default()
        };

        Ok(self
            .operator
            .read_options(&path, read_options)
            .map_err(|err|
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Partially reading file `{path}` failed in the backend. Please check if the given path is correct.",
                    err,
                )
                .attach_context("path", path)
                .attach_context("type", tpe.to_string())
                .attach_context("id", id.to_string())
                .attach_context("offset", offset.to_string())
                .attach_context("length", length.to_string())
            )?
            .to_bytes())
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.path(tpe, id)
    }
}

impl WriteBackend for OpenDALBackend {
    /// Create a repository on the backend.
    fn create(&self) -> RusticResult<()> {
        trace!("creating repo at {:?}", self.location());

        for tpe in ALL_FILE_TYPES {
            let path = tpe.dirname().to_string() + "/";
            self.operator
                .create_dir(&path)
                .map_err(|err|
                    RusticError::with_source(
                        ErrorKind::Backend,
                        "Creating directory `{path}` failed in the backend `{location}`. Please check if the given path is correct.",
                        err,
                    )
                    .attach_context("path", path)
                    .attach_context("location", self.location())
                    .attach_context("type", tpe.to_string())
                )?;
        }
        // creating 256 dirs can be slow on remote backends, hence we parallelize it.
        (0u8..=255)
            .into_par_iter()
            .try_for_each(|i| {
                let path = UnixPathBuf::from("data")
                        .join(hex::encode([i]))
                        .to_string_lossy()
                        .to_string()
                        + "/";

                self.operator.create_dir(&path).map_err(|err|
                    RusticError::with_source(
                        ErrorKind::Backend,
                        "Creating directory `{path}` failed in the backend `{location}`. Please check if the given path is correct.",
                        err,
                    )
                    .attach_context("path", path)
                    .attach_context("location", self.location())
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
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        buf: Bytes,
    ) -> RusticResult<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        _ = self.operator.write(&filename, buf).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Writing file `{path}` failed in the backend. Please check if the given path is correct.",
                err,
            )
            .attach_context("path", filename)
            .attach_context("type", tpe.to_string())
            .attach_context("id", id.to_string())
        })?;

        Ok(())
    }

    /// Remove the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let filename = self.path(tpe, id);
        self.operator.delete(&filename).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Backend,
                "Deleting file `{path}` failed in the backend. Please check if the given path is correct.",
                err,
            )
            .attach_context("path", filename)
            .attach_context("type", tpe.to_string())
            .attach_context("id", id.to_string())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rstest::rstest;
    use serde::Deserialize;
    use std::{fs, path::PathBuf};

    #[rstest]
    #[case("10kB,10MB", Throttle{bandwidth:10_000, burst:10_000_000})]
    #[case("10 kB,10  MB", Throttle{bandwidth:10_000, burst:10_000_000})]
    #[case("10kB, 10MB", Throttle{bandwidth:10_000, burst:10_000_000})]
    #[case(" 10kB,   10MB", Throttle{bandwidth:10_000, burst:10_000_000})]
    #[case("10kiB,10MiB", Throttle{bandwidth:10_240, burst:10_485_760})]
    fn correct_throttle(#[case] input: &str, #[case] expected: Throttle) {
        assert_eq!(Throttle::from_str(input).unwrap(), expected);
    }

    #[rstest]
    #[case("")]
    #[case("10kiB")]
    #[case("no_number,10MiB")]
    #[case("10kB;10MB")]
    fn invalid_throttle(#[case] input: &str) {
        assert!(Throttle::from_str(input).is_err());
    }

    #[rstest]
    fn new_opendal_backend(
        #[files("tests/fixtures/opendal/*.toml")] test_case: PathBuf,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct TestCase {
            path: String,
            options: BTreeMap<String, String>,
        }

        let test: TestCase = toml::from_str(&fs::read_to_string(test_case)?)?;

        _ = OpenDALBackend::new(test.path, test.options)?;
        Ok(())
    }
}
