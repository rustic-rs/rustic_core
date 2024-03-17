use std::str::FromStr;
use std::time::Duration;

use anyhow::Result;
use backoff::{backoff::Backoff, ExponentialBackoff, ExponentialBackoffBuilder};
use bytes::Bytes;
use log::{trace, warn};
use reqwest::{
    blocking::{Client, ClientBuilder, Response},
    header::{HeaderMap, HeaderValue},
    Url,
};
use serde::Deserialize;

use crate::error::RestErrorKind;

use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

mod consts {
    /// Default number of retries
    pub(super) const DEFAULT_RETRY: usize = 5;
}

/// `ValidateResponse` to add user-defined method `validate` on a response
///
/// This trait is used to add a method `validate` on a response to check for errors
/// and treat them as permanent or transient based on the status code of the response.
///
/// It returns a result with the response if it is not an error, otherwise it returns
/// the associated error.
pub trait ValidateResponse {
    /// The error type that is returned if the response is an error
    type Error;

    /// Check a response for an error and treat it as permanent or transient
    ///
    /// # Errors
    ///
    /// If the response is an error, it will return an error of type [`Self::Error`]
    ///
    /// # Returns
    ///
    /// The response if it is not an error or an error of type [`Self::Error`] if it is an error
    fn validate(self) -> Result<Self, Self::Error>
    where
        Self: Sized;
}

impl ValidateResponse for Response {
    type Error = backoff::Error<reqwest::Error>;

    /// Check reqwest Response for error and treat errors as permanent or transient
    /// based on the status code of the response
    ///
    /// # Errors
    ///
    /// If the response is an error, it will return an [`reqwest::Error`]
    ///
    /// # Returns
    ///
    /// The [`Response`] if it is not an error
    fn validate(self) -> Result<Self, Self::Error> {
        match self.error_for_status() {
            Ok(t) => Ok(t),
            // Note: status() always give Some(_) as it is called from a Response
            Err(err) => {
                let Some(status) = err.status() else {
                    return Err(backoff::Error::Permanent(err));
                };

                if status.is_client_error() {
                    Err(backoff::Error::Permanent(err))
                } else {
                    Err(backoff::Error::Transient {
                        err,
                        retry_after: None,
                    })
                }
            }
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
            max_retries: consts::DEFAULT_RETRY,
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
    /// * [`RestErrorKind::UrlParsingFailed`] - If the url could not be parsed.
    /// * [`RestErrorKind::BuildingClientFailed`] - If the client could not be built.
    ///
    /// [`RestErrorKind::UrlParsingFailed`]: RestErrorKind::UrlParsingFailed
    /// [`RestErrorKind::BuildingClientFailed`]: RestErrorKind::BuildingClientFailed
    pub fn new(
        url: impl AsRef<str>,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let url = url.as_ref();
        let url = if url.ends_with('/') {
            Url::parse(url).map_err(RestErrorKind::UrlParsingFailed)?
        } else {
            // add a trailing '/' if there is none
            let mut url = url.to_string();
            url.push('/');
            Url::parse(&url).map_err(RestErrorKind::UrlParsingFailed)?
        };

        let mut headers = HeaderMap::new();
        _ = headers.insert("User-Agent", HeaderValue::from_static("rustic"));

        let mut client = ClientBuilder::new()
            .default_headers(headers)
            .timeout(Duration::from_secs(600)) // set default timeout to 10 minutes (we can have *large* packfiles)
            .build()
            .map_err(RestErrorKind::BuildingClientFailed)?;
        let mut backoff = LimitRetryBackoff::default();

        for (option, value) in options {
            if option == "retry" {
                let max_retries = match value.as_str() {
                    "false" | "off" => 0,
                    "default" => consts::DEFAULT_RETRY,
                    _ => usize::from_str(&value)
                        .map_err(|_| RestErrorKind::NotSupportedForRetry(value))?,
                };
                backoff.max_retries = max_retries;
            } else if option == "timeout" {
                let timeout = match humantime::Duration::from_str(&value) {
                    Ok(val) => val,
                    Err(e) => return Err(RestErrorKind::CouldNotParseDuration(e).into()),
                };
                client = match ClientBuilder::new().timeout(*timeout).build() {
                    Ok(val) => val,
                    Err(err) => return Err(RestErrorKind::BuildingClientFailed(err).into()),
                };
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
    /// If the url could not be created.
    fn url(&self, tpe: FileType, id: &Id) -> Result<Url> {
        let id_path = if tpe == FileType::Config {
            "config".to_string()
        } else {
            let hex_id = id.to_hex();
            let mut path = tpe.dirname().to_string();
            path.push('/');
            path.push_str(&hex_id);
            path
        };
        Ok(self
            .url
            .join(&id_path)
            .map_err(RestErrorKind::JoiningUrlFailed)?)
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
    /// * [`RestErrorKind::JoiningUrlFailed`] - If the url could not be created.
    ///
    /// # Notes
    ///
    /// The returned list is sorted by id.
    ///
    /// # Returns
    ///
    /// A vector of tuples containing the id and size of the files.
    ///
    /// [`RestErrorKind::JoiningUrlFailed`]: RestErrorKind::JoiningUrlFailed
    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        // format which is delivered by the REST-service
        #[derive(Deserialize)]
        struct ListEntry {
            name: String,
            size: u32,
        }

        trace!("listing tpe: {tpe:?}");
        let url = if tpe == FileType::Config {
            self.url
                .join("config")
                .map_err(RestErrorKind::JoiningUrlFailed)?
        } else {
            let mut path = tpe.dirname().to_string();
            path.push('/');
            self.url
                .join(&path)
                .map_err(RestErrorKind::JoiningUrlFailed)?
        };

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
                    .validate()?
                    .json::<Option<Vec<ListEntry>>>()? // use Option to be handle null json value
                    .unwrap_or_default();
                Ok(list
                    .into_iter()
                    .filter_map(|i| match Id::from_hex(&i.name) {
                        Ok(id) => Some((id, i.size)),
                        Err(_) => None,
                    })
                    .collect())
            },
            notify,
        )
        .map_err(|e| RestErrorKind::BackoffError(e).into())
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
    /// * [`reqwest::Error`] - If the request failed.
    /// * [`RestErrorKind::BackoffError`] - If the backoff failed.
    ///
    /// [`RestErrorKind::BackoffError`]: RestErrorKind::BackoffError
    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}");
        let url = self.url(tpe, id)?;
        Ok(backoff::retry_notify(
            self.backoff.clone(),
            || Ok(self.client.get(url.clone()).send()?.validate()?.bytes()?),
            notify,
        )
        .map_err(RestErrorKind::BackoffError)?)
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
    /// * [`RestErrorKind::BackoffError`] - If the backoff failed.
    ///
    /// [`RestErrorKind::BackoffError`]: RestErrorKind::BackoffError
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        trace!("reading tpe: {tpe:?}, id: {id}, offset: {offset}, length: {length}");
        let offset2 = offset + length - 1;
        let header_value = format!("bytes={offset}-{offset2}");
        let url = self.url(tpe, id)?;
        Ok(backoff::retry_notify(
            self.backoff.clone(),
            || {
                Ok(self
                    .client
                    .get(url.clone())
                    .header("Range", header_value.clone())
                    .send()?
                    .validate()?
                    .bytes()?)
            },
            notify,
        )
        .map_err(RestErrorKind::BackoffError)?)
    }
}

impl WriteBackend for RestBackend {
    /// Creates a new file.
    ///
    /// # Errors
    ///
    /// * [`RestErrorKind::BackoffError`] - If the backoff failed.
    ///
    /// [`RestErrorKind::BackoffError`]: RestErrorKind::BackoffError
    fn create(&self) -> Result<()> {
        let url = self
            .url
            .join("?create=true")
            .map_err(RestErrorKind::JoiningUrlFailed)?;
        Ok(backoff::retry_notify(
            self.backoff.clone(),
            || {
                _ = self.client.post(url.clone()).send()?.validate()?;
                Ok(())
            },
            notify,
        )
        .map_err(RestErrorKind::BackoffError)?)
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
    /// * [`RestErrorKind::BackoffError`] - If the backoff failed.
    ///
    /// [`RestErrorKind::BackoffError`]: RestErrorKind::BackoffError
    fn write_bytes(&self, tpe: FileType, id: &Id, _cacheable: bool, buf: Bytes) -> Result<()> {
        trace!("writing tpe: {:?}, id: {}", &tpe, &id);
        let req_builder = self.client.post(self.url(tpe, id)?).body(buf);
        Ok(backoff::retry_notify(
            self.backoff.clone(),
            || {
                // Note: try_clone() always gives Some(_) as the body is Bytes which is clonable
                    .validate()?;
                Ok(())
            },
            notify,
        )
        .map_err(RestErrorKind::BackoffError)?)
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
    /// * [`RestErrorKind::BackoffError`] - If the backoff failed.
    ///
    /// [`RestErrorKind::BackoffError`]: RestErrorKind::BackoffError
    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> Result<()> {
        trace!("removing tpe: {:?}, id: {}", &tpe, &id);
        let url = self.url(tpe, id)?;
        Ok(backoff::retry_notify(
            self.backoff.clone(),
            || {
                _ = self.client.delete(url.clone()).send()?.validate()?;
                Ok(())
            },
            notify,
        )
        .map_err(RestErrorKind::BackoffError)?)
    }
}
