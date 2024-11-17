use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;
use log::{trace, warn};
use reqwest::{
    blocking::{Client, ClientBuilder},
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

mod backon_extension {
    use std::time::Duration;

    use backon::{BlockingRetryable, ExponentialBuilder};

    use super::constants;

    /// Trait to implement on error types to combine with [`LimitRetry::retry_notify`].
    pub(super) trait NotifyWhenRetry {
        fn when_retry(&self) -> bool {
            // by default always retry
            true
        }
        #[allow(unused_variables)]
        fn notify(&self, dur: Duration) {}
    }

    /// A backon::backoff extension that limits the number of retries
    #[derive(Debug)]
    pub struct LimitRetry {
        max_retries: usize,
    }

    impl Default for LimitRetry {
        fn default() -> Self {
            Self {
                max_retries: constants::DEFAULT_RETRY,
            }
        }
    }

    /// We need to impl [`Clone`] manually because [backon::ExponentialBuilder] doesn't.
    impl Clone for LimitRetry {
        fn clone(&self) -> Self {
            Self {
                max_retries: self.max_retries,
            }
        }
    }

    impl LimitRetry {
        pub fn new(max_retries: usize) -> Self {
            Self { max_retries }
        }

        pub fn set_max_retries(&mut self, max_retries: usize) {
            self.max_retries = max_retries;
        }

        fn builder(&self) -> ExponentialBuilder {
            // backon doesn't allow us to specify `None` for `max_delay`
            // see <https://github.com/Xuanwo/backon/pull/160>
            ExponentialBuilder::default()
                .with_max_delay(Duration::MAX) // no maximum elapsed time; we count number of retries
                .with_max_times(self.max_retries)
        }

        pub fn retry_notify<F, T, E>(&self, op: F) -> Result<T, E>
        where
            F: FnMut() -> Result<T, E>,
            E: NotifyWhenRetry,
        {
            let mut retry = op.retry(self.builder());
            retry = retry.notify(E::notify);
            retry = retry.when(E::when_retry);
            retry.call()
        }
    }
}

impl backon_extension::NotifyWhenRetry for reqwest::Error {
    /// Heuristic to decide if the error could be recovered by retrying or not.
    ///
    /// If the error could be recovered by a retry: return `true`.
    ///
    /// Else return `false` and the combined backoff will stop early.
    fn when_retry(&self) -> bool {
        match self.status() {
            Some(status_code) => !status_code.is_client_error(), // do no retry if client error
            None => true,                                        // else retry
        }
    }

    /// Notify function for backon in case of error
    ///
    /// # Arguments
    ///
    /// * `err` - The error that occurred
    /// * `duration` - The duration of the backoff
    fn notify(&self, duration: Duration) {
        warn!("Error {self} at {duration:?}, retrying");
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
    retry_handler: backon_extension::LimitRetry,
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

        let mut backoff_generator = backon_extension::LimitRetry::default();

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
                backoff_generator.set_max_retries(max_retries);
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
            retry_handler: backoff_generator,
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
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
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
                Ok(self
                    .client
                    .get(url.clone())
                    .send()?
                    .error_for_status()?
                    .bytes()?)
            })
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
                Ok(self
                    .client
                    .get(url.clone())
                    .header("Range", header_value.clone())
                    .send()?
                    .error_for_status()?
                    .bytes()?)
            })
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
                _ = self.client.post(url.clone()).send()?.error_for_status()?;
                Ok(())
            })
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
                // Note: try_clone() always gives Some(_) as the body is Bytes which is cloneable
                _ = req_builder
                    .try_clone()
                    .unwrap()
                    .send()?
                    .error_for_status()?;
                Ok(())
            })
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
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

        self.retry_handler
            .retry_notify::<_, _, reqwest::Error>(|| {
                _ = self.client.delete(url.clone()).send()?.error_for_status()?;
                Ok(())
            })
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Backoff failed, please check the logs for more information.",
                    err,
                )
            })
    }
}
