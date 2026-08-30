use std::str::FromStr;
use std::{collections::BTreeMap, io::Read};

use jiff::SignedDuration;
use log::warn;
use reqwest::{
    Client, ClientBuilder,
    header::{HeaderMap, HeaderValue},
};

use rustic_core::{ErrorKind, RusticError, RusticResult};

pub(super) mod constants {
    use std::time::Duration;

    /// Default timeout for the client
    /// This is set to 10 minutes
    pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_mins(10);
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

pub fn reqwest_client(options: &BTreeMap<String, String>) -> RusticResult<Client> {
    let mut headers = HeaderMap::new();
    _ = headers.insert("User-Agent", HeaderValue::from_static("rustic"));

    // set default timeout to 10 minutes (we can have *large* packfiles)
    let mut timeout = constants::DEFAULT_TIMEOUT;

    let mut client_builder = ClientBuilder::new().default_headers(headers);

    for (option, value) in options {
        if option == "timeout" {
            timeout = SignedDuration::from_str(value).map_err(|err| {
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
            client_builder = client_builder.add_root_certificate(get_cacert(value)?);
        } else if option == "tls-client-cert" {
            client_builder = client_builder.identity(get_tls_client_cert(value)?);
        } else if option == "http-insecure-tls" {
            match value.parse() {
                Err(err) => warn!("cannot use value for `http-insecure-tls`: {err}"),
                Ok(insecure_tls) => {
                    client_builder = client_builder.danger_accept_invalid_certs(insecure_tls);
                }
            }
        } else if option == "http-referer" {
            match value.parse() {
                Err(err) => warn!("cannot use value for `http-referer`: {err}"),
                Ok(referer) => {
                    client_builder = client_builder.referer(referer);
                }
            }
        }
    }

    let client = client_builder.timeout(timeout).build().map_err(|err| {
        RusticError::with_source(ErrorKind::Backend, "Failed to build HTTP client", err)
    })?;

    Ok(client)
}
