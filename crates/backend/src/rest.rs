use std::io::Read;
use std::str::FromStr;
use std::time::Duration;

use backon::{BlockingRetryable, ExponentialBuilder};
use bytes::Bytes;
use jiff::SignedDuration;
use log::{trace, warn};
use reqwest::{
    Url,
    blocking::{Client, ClientBuilder},
    header::{HeaderMap, HeaderValue},
};
use serde::Deserialize;

use rustic_core::{ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend};

/// joining URL failed on: `{0}`
#[derive(thiserror::Error, Clone, Copy, Debug, displaydoc::Display)]
pub struct JoiningUrlFailedError(url::ParseError);

pub(super) mod constants {
    use std::time::Duration;

    /// Default number of retries
    pub(super) const DEFAULT_RETRY: usize = 5;

    /// Default timeout for the client
    /// This is set to 10 minutes
    pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
}

fn construct_backoff_error(err: reqwest::Error) -> Box<RusticError> {
    RusticError::with_source(
        ErrorKind::Backend,
        "Backoff failed, please check the logs for more information.",
        err,
    )
}

fn read_file_contents(log_name: &str, path: &str) -> RusticResult<Vec<u8>> {
    let mut buf = Vec::new();
    let _ = std::fs::File::open(path)
        .map_err(|err| {
            RusticError::with_source(
                ErrorKind::InvalidInput,
                "Cannot open {log_name} `{path}`",
                err,
            )
            .attach_context("path", path)
            .attach_context("log_name", log_name)
        })?
        .read_to_end(&mut buf)
        .map_err(|err| {
            RusticError::with_source(
                ErrorKind::InvalidInput,
                "Cannot read {log_name} `{path}`",
                err,
            )
            .attach_context("path", path)
            .attach_context("log_name", log_name)
        })?;
    Ok(buf)
}

fn get_cacert(value: &str) -> RusticResult<reqwest::Certificate> {
    let buf = read_file_contents("cacert", value)?;
    reqwest::Certificate::from_pem(&buf).map_err(|err| {
        RusticError::with_source(
            ErrorKind::InvalidInput,
            "Cannot parse cacert `{value}`",
            err,
        )
        .attach_context("value", value)
    })
}

fn get_tls_client_cert(value: &str) -> RusticResult<reqwest::Identity> {
    let buf = read_file_contents("tls-client-cert", value)?;
    reqwest::Identity::from_pem(&buf).map_err(|err| {
        RusticError::with_source(
            ErrorKind::InvalidInput,
            "Cannot parse tls-client-cert `{value}`",
            err,
        )
        .attach_context("value", value)
    })
}

/// A backend implementation that uses REST to access the backend.
#[derive(Clone, Debug)]
pub struct RestBackend {
    /// The url of the backend.
    url: Url,
    /// The client to use.
    client: Client,
    /// The ``BackoffBuilder`` we use
    backoff: ExponentialBuilder,
}

impl RestBackend {
    /// Call the given operation retrying non-permanent errors and giving warnings for failed operations
    ///
    /// ## Permanent/non-permanent errors
    ///
    /// -  `client_error` are considered permanent
    /// - others are not, and are subject to retry
    ///
    /// ## Returns
    ///
    /// The operation result
    /// or the last error (permanent or not) that occurred.
    fn retry_notify<F, T>(&self, op: F) -> Result<T, reqwest::Error>
    where
        F: FnMut() -> Result<T, reqwest::Error>,
    {
        op.retry(self.backoff)
            .when(|err| {
                err.status().map_or(
                    true,                                         // retry
                    |status_code| !status_code.is_client_error(), // do not retry if `is_client_error`
                )
            })
            .notify(|err, duration| warn!("Error {err} at {duration:?}, retrying"))
            .call()
    }

    /// Create a new [`RestBackend`] from a given url.
    ///
    /// # Arguments
    ///
    /// * `url` - The url to create the [`RestBackend`] from.
    ///
    /// # Errors
    ///
    /// * If the url could not be parsed.
    /// * If the client could not be built.
    pub fn new(
        url: impl AsRef<str>,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> RusticResult<Self> {
        let url = url.as_ref().to_string();

        let url = if url.ends_with('/') {
            url
        } else {
            // add a trailing '/' if there is none
            let mut url = url;
            url.push('/');
            url
        };

        let url = Url::parse(&url).map_err(|err| {
            RusticError::with_source(ErrorKind::InvalidInput, "URL `{url}` parsing failed", err)
                .attach_context("url", url)
        })?;

        let mut headers = HeaderMap::new();
        _ = headers.insert("User-Agent", HeaderValue::from_static("rustic"));

        // set default timeout to 10 minutes (we can have *large* packfiles)
        let mut timeout = constants::DEFAULT_TIMEOUT;

        let mut client_builder = ClientBuilder::new().default_headers(headers);

        // backon doesn't allow us to specify `None` for `max_delay`
        // see <https://github.com/Xuanwo/backon/pull/160>
        let mut backoff = ExponentialBuilder::default()
            .with_max_delay(Duration::MAX) // no maximum elapsed time; we count number of retries
            .with_max_times(constants::DEFAULT_RETRY);

        // FIXME: If we have multiple times the same option, this could lead to unexpected behavior
        for (option, value) in options {
            if option == "retry" {
                let max_retries = match value.as_str() {
                    "false" | "off" => 0,
                    "default" => constants::DEFAULT_RETRY,
                    _ => usize::from_str(&value).map_err(|err| {
                        RusticError::with_source(
                            ErrorKind::InvalidInput,
                            "Cannot parse value `{value}`, invalid value for option `{option}`.",
                            err,
                        )
                        .attach_context("value", value)
                        .attach_context("option", "retry")
                    })?,
                };
                backoff = backoff.with_max_times(max_retries);
            } else if option == "timeout" {
                timeout = SignedDuration::from_str(&value).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Could not parse value `{value}` as duration. Invalid value for option `{option}`.",
                        err,
                    )
                    .attach_context("value", value)
                    .attach_context("option", "timeout")
                })?.try_into()
                // ignore conversation errors, but print out warning
                .inspect_err(|err| warn!("cannot use timeout: {err}"))
                .unwrap_or_default();
            } else if option == "cacert" {
                client_builder = client_builder.add_root_certificate(get_cacert(&value)?);
            } else if option == "tls-client-cert" {
                client_builder = client_builder.identity(get_tls_client_cert(&value)?);
            }
        }

        let client = client_builder.timeout(timeout).build().map_err(|err| {
            RusticError::with_source(ErrorKind::Backend, "Failed to build HTTP client", err)
        })?;

        Ok(Self {
            url,
            client,
            backoff,
        })
    }

    /// Returns the url for a given type and id.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * If the url could not be joined/created.
    fn url(&self, tpe: FileType, id: &Id) -> Result<Url, JoiningUrlFailedError> {
        let id_path = if tpe == FileType::Config {
            "config".to_string()
        } else {
            let hex_id = id.to_hex();
            let mut path = tpe.dirname().to_string();
            path.push('/');
            path.push_str(&hex_id);
            path
        };

        self.url.join(&id_path).map_err(JoiningUrlFailedError)
    }
}

impl ReadBackend for RestBackend {
    /// Returns the location of the backend.
    fn location(&self) -> String {
        let mut location = "rest:".to_string();
        let mut url = self.url.clone();
        if url.password().is_some() {
            url.set_password(Some("***")).unwrap();
        }
        location.push_str(url.as_str());
        location
    }

    /// Returns a list of all files of a given type with their size.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list.
    ///
    /// # Errors
    ///
    /// * If the url could not be created.
    ///
    /// # Notes
    ///
    /// The returned list is sorted by id.
    ///
    /// # Returns
    ///
    /// A vector of tuples containing the id and size of the files.
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        // format which is delivered by the REST-service
        #[derive(Deserialize)]
        struct ListEntry {
            name: String,
            size: u32,
        }

        trace!("listing tpe: {tpe:?}");

        // TODO: Explain why we need special handling here
        let path = if tpe == FileType::Config {
            "config".to_string()
        } else {
            let mut path = tpe.dirname().to_string();
            path.push('/');
            path
        };

        let url = self.url.join(&path).map_err(|err| {
            RusticError::with_source(ErrorKind::Internal, "Joining URL `{url}` failed", err)
                .attach_context("url", self.url.as_str())
                .attach_context("tpe", tpe.to_string())
                .attach_context("tpe_dir", tpe.dirname().to_string())
        })?;

        self.retry_notify(|| {
            if tpe == FileType::Config {
                return Ok(
                    if self.client.head(url.clone()).send()?.status().is_success() {
                        vec![(Id::default(), 0)]
                    } else {
                        Vec::new()
                    },
                );
            }

            let list = self
                .client
                .get(url.clone())
                .header("Accept", "application/vnd.x.restic.rest.v2")
                .send()?
                .error_for_status()?
                .json::<Option<Vec<ListEntry>>>()? // use Option to be handle null json value
                .unwrap_or_default();

            Ok(list
                .into_iter()
                .filter_map(|i| match i.name.parse::<Id>() {
                    Ok(id) => Some((id, i.size)),
                    Err(_) => None,
                })
                .collect())
        })
        .map_err(construct_backoff_error)
    }

    /// Returns the content of a file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    ///
    /// # Errors
    ///
    /// * If the request failed.
    /// * If the backoff failed.
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");

        let url = self
            .url(tpe, id)
            .map_err(|err| construct_join_url_error(err, tpe, id, &self.url))?;

        self.retry_notify(|| {
            self.client
                .get(url.clone())
                .send()?
                .error_for_status()?
                .bytes()
        })
        .map_err(construct_backoff_error)
    }

    /// Returns a part of the content of a file.
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
    /// * If the backoff failed.
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let offset2 = offset + length - 1;
        let header_value = format!("bytes={offset}-{offset2}");
        let url = self.url(tpe, id).map_err(|err| {
            RusticError::with_source(ErrorKind::Internal, "Joining URL `{url}` failed", err)
                .attach_context("url", self.url.as_str())
                .attach_context("tpe", tpe.to_string())
                .attach_context("tpe_dir", tpe.dirname().to_string())
                .attach_context("id", id.to_string())
        })?;

        self.retry_notify(|| {
            self.client
                .get(url.clone())
                .header("Range", header_value.clone())
                .send()?
                .error_for_status()?
                .bytes()
        })
        .map_err(construct_backoff_error)
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        // For REST backends, return the URL path that could be used for warmup
        // Though warmup is typically handled by the REST server itself
        self.url
            .join(&format!("{}/{}", tpe.dirname(), id))
            .unwrap()
            .to_string()
    }
}

fn construct_join_url_error(
    err: JoiningUrlFailedError,
    tpe: FileType,
    id: &Id,
    self_url: &Url,
) -> Box<RusticError> {
    RusticError::with_source(ErrorKind::Internal, "Joining URL `{url}` failed", err)
        .attach_context("url", self_url.as_str())
        .attach_context("tpe", tpe.to_string())
        .attach_context("tpe_dir", tpe.dirname().to_string())
        .attach_context("id", id.to_string())
}

impl WriteBackend for RestBackend {
    /// Creates a new file.
    ///
    /// # Errors
    ///
    /// * If the backoff failed.
    fn create(&self) -> RusticResult<()> {
        let url = self.url.join("?create=true").map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Joining URL `{url}` with `{join_input}` failed",
                err,
            )
            .attach_context("url", self.url.as_str())
            .attach_context("join_input", "?create=true")
        })?;

        self.retry_notify(|| {
            _ = self.client.post(url.clone()).send()?.error_for_status()?;
            Ok(())
        })
        .map_err(construct_backoff_error)
    }

    /// Writes bytes to the given file.
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
    /// * If the backoff failed.
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        buf: Bytes,
    ) -> RusticResult<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let req_builder = self
            .client
            .post(
                self.url(tpe, id)
                    .map_err(|err| construct_join_url_error(err, tpe, id, &self.url))?,
            )
            .body(buf);

        self.retry_notify(|| {
            // Note: try_clone() always gives Some(_) as the body is Bytes which is cloneable
            _ = req_builder
                .try_clone()
                .unwrap()
                .send()?
                .error_for_status()?;
            Ok(())
        })
        .map_err(construct_backoff_error)
    }

    /// Removes the given file.
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file.
    /// * `id` - The id of the file.
    /// * `cacheable` - Whether the file is cacheable.
    ///
    /// # Errors
    ///
    /// * If the backoff failed.
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let url = self
            .url(tpe, id)
            .map_err(|err| construct_join_url_error(err, tpe, id, &self.url))?;

        self.retry_notify(|| {
            _ = self.client.delete(url.clone()).send()?.error_for_status()?;
            Ok(())
        })
        .map_err(construct_backoff_error)
    }
}
