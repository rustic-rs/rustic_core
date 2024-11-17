use std::str::FromStr;
use std::time::Duration;

use backoff::{backoff::Backoff, ExponentialBackoff, ExponentialBackoffBuilder};
use bytes::Bytes;
use log::{trace, warn};
use reqwest::{
    blocking::{Client, ClientBuilder, Response},
    header::{HeaderMap, HeaderValue},
    Url,
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

// trait CheckError to add user-defined method check_error on Response
pub(crate) trait CheckError {
    /// Check reqwest Response for error and treat errors as permanent or transient
    fn check_error(self) -> Result<Response, backoff::Error<reqwest::Error>>;
}

impl CheckError for Response {
    /// Check reqwest Response for error and treat errors as permanent or transient
    ///
    /// # Errors
    ///
    /// If the response is an error, it will return an error of type Error<reqwest::Error>
    ///
    /// # Returns
    ///
    /// The response if it is not an error
    fn check_error(self) -> Result<Response, backoff::Error<reqwest::Error>> {
        match self.error_for_status() {
            Ok(t) => Ok(t),
            // Note: status() always give Some(_) as it is called from a Response
            Err(err) if err.status().unwrap().is_client_error() => {
                Err(backoff::Error::Permanent(err))
            }
            Err(err) => Err(backoff::Error::Transient {
                err,
                retry_after: None,
            }),
        }
    }
}

/// A backoff implementation that limits the number of retries
#[derive(Clone, Debug)]
struct LimitRetryBackoff {
    /// The maximum number of retries
    max_retries: usize,
    /// The current number of retries
    retries: usize,
    /// The exponential backoff
    exp: ExponentialBackoff,
}

impl Default for LimitRetryBackoff {
    fn default() -> Self {
        Self {
            max_retries: constants::DEFAULT_RETRY,
            retries: 0,
            exp: ExponentialBackoffBuilder::new()
                .with_max_elapsed_time(None) // no maximum elapsed time; we count number of retires
                .build(),
        }
    }
}

impl Backoff for LimitRetryBackoff {
    /// Returns the next backoff duration.
    ///
    /// # Notes
    ///
    /// If the number of retries exceeds the maximum number of retries, it returns None.
    fn next_backoff(&mut self) -> Option<Duration> {
        self.retries += 1;
        if self.retries > self.max_retries {
            None
        } else {
            self.exp.next_backoff()
        }
    }

    /// Resets the backoff to the initial state.
    fn reset(&mut self) {
        self.retries = 0;
        self.exp.reset();
    }
}

/// A backend implementation that uses REST to access the backend.
#[derive(Clone, Debug)]
pub struct RestBackend {
    /// The url of the backend.
    url: Url,
    /// The client to use.
    client: Client,
    /// The backoff implementation to use.
    backoff: LimitRetryBackoff,
}

/// Notify function for backoff in case of error
///
/// # Arguments
///
/// * `err` - The error that occurred
/// * `duration` - The duration of the backoff
// We need to pass the error by value to satisfy the signature of the notify function
// for handling errors in backoff
#[allow(clippy::needless_pass_by_value)]
fn notify(err: reqwest::Error, duration: Duration) {
    warn!("Error {err} at {duration:?}, retrying");
}

impl RestBackend {
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

        let mut client = ClientBuilder::new()
            .default_headers(headers)
            .timeout(constants::DEFAULT_TIMEOUT) // set default timeout to 10 minutes (we can have *large* packfiles)
            .build()
            .map_err(|err| {
                RusticError::with_source(ErrorKind::Backend, "Failed to build HTTP client", err)
            })?;
        let mut backoff = LimitRetryBackoff::default();

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
                backoff.max_retries = max_retries;
            } else if option == "timeout" {
                let timeout = humantime::Duration::from_str(&value).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::InvalidInput,
                        "Could not parse value `{value}` as `humantime` duration. Invalid value for option `{option}`.",
                        err,
                    )
                    .attach_context("value", value)
                    .attach_context("option", "timeout")
                })?;

                client = ClientBuilder::new()
                    .timeout(*timeout)
                    .build()
                    .map_err(|err| {
                        RusticError::with_source(
                            ErrorKind::Backend,
                            "Failed to build HTTP client",
                            err,
                        )
                    })?;
            }
        }

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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
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
                    .check_error()?
                    .json::<Option<Vec<ListEntry>>>()? // use Option to be handle null json value
                    .unwrap_or_default();
                Ok(list
                    .into_iter()
                    .filter_map(|i| match i.name.parse::<Id>() {
                        Ok(id) => Some((id, i.size)),
                        Err(_) => None,
                    })
                    .collect())
            },
            notify,
        )
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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
                Ok(self
                    .client
                    .get(url.clone())
                    .send()?
                    .check_error()?
                    .bytes()?)
            },
            notify,
        )
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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
                Ok(self
                    .client
                    .get(url.clone())
                    .header("Range", header_value.clone())
                    .send()?
                    .check_error()?
                    .bytes()?)
            },
            notify,
        )
        .map_err(construct_backoff_error)
    }

    /// [`RestBackend`] uses `reqwest` which blocking implementation
    /// uses an `async` runtime under the hood.
    ///
    /// When implementing `rustic_core` using this backend in some `async` features will not work.
    ///
    /// see <https://github.com/rustic-rs/rustic/issues/1181>
    fn is_async_incompatible(&self) -> bool {
        true
    }
}

fn construct_backoff_error(err: backoff::Error<reqwest::Error>) -> Box<RusticError> {
    RusticError::with_source(
        ErrorKind::Backend,
        "Backoff failed, please check the logs for more information.",
        err,
    )
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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
                _ = self.client.post(url.clone()).send()?.check_error()?;
                Ok(())
            },
            notify,
        )
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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
                // Note: try_clone() always gives Some(_) as the body is Bytes which is cloneable
                _ = req_builder.try_clone().unwrap().send()?.check_error()?;
                Ok(())
            },
            notify,
        )
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

        backoff::retry_notify(
            self.backoff.clone(),
            || {
                _ = self.client.delete(url.clone()).send()?.check_error()?;
                Ok(())
            },
            notify,
        )
        .map_err(construct_backoff_error)
    }
}
