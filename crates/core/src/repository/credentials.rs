use std::fs::File;
use std::io::BufRead;
use std::{io::BufReader, path::Path, path::PathBuf};

use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[cfg(feature = "clap")]
use clap::ValueHint;

use crate::{CommandInput, ErrorKind, RusticError, RusticResult, repofile::MasterKey};

#[derive(Debug, Clone)]
/// Credential to open a repository
pub enum Credentials {
    /// credentials are given directly by specifying the masterkey
    Masterkey(MasterKey),
    /// credentials are given by a password
    Password(String),
}

impl Credentials {
    /// Convenience constructor for password
    pub fn password(pass: impl AsRef<str>) -> Self {
        Self::Password(pass.as_ref().to_string())
    }
}

/// Options for using and opening a [`Repository`]
#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Default, Debug, Deserialize, Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into, strip_option)]
#[non_exhaustive]
pub struct CredentialOptions {
    /// masterkey to use
    #[cfg_attr(
        feature = "clap",
        clap(long, global = true, env = "RUSTIC_KEY", hide_env_values = true)
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub key: Option<String>,

    /// File to read the masterkey from
    #[cfg_attr(
        feature = "clap",
        clap(
            long,
            global = true,
            env = "RUSTIC_KEY_FILE",
            conflicts_with_all = &["password", "password_file", "password_command"],
            value_hint = ValueHint::FilePath,
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub key_file: Option<PathBuf>,

    /// Command to read the masterkey from. Key is read from stdout
    #[cfg_attr(
        feature = "clap",
        clap(
            long,
            global = true,
            env = "RUSTIC_KEY_COMMAND",
            conflicts_with_all = &["password", "password_file", "password_command"],
            value_hint = ValueHint::FilePath,
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub key_command: Option<CommandInput>,

    /// Password for the repository
    ///
    /// # Warning
    ///
    /// * Using --password can reveal the password in the process list!
    #[cfg_attr(
        feature = "clap",
        clap(long, global = true, env = "RUSTIC_PASSWORD", hide_env_values = true)
    )]
    // TODO: Security related: use `secrecy` library (#663)
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub password: Option<String>,

    /// File to read the password from
    #[cfg_attr(
        feature = "clap",
        clap(
            short,
            long,
            global = true,
            env = "RUSTIC_PASSWORD_FILE",
            conflicts_with = "password",
            value_hint = ValueHint::FilePath,
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub password_file: Option<PathBuf>,

    /// Command to read the password from. Password is read from stdout
    #[cfg_attr(feature = "clap", clap(
        long,
        global = true,
        env = "RUSTIC_PASSWORD_COMMAND",
        conflicts_with_all = &["password", "password_file"],
    ))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub password_command: Option<CommandInput>,
}

impl CredentialOptions {
    /// Evaluates the credentials given by the repository options
    ///
    /// # Errors
    ///
    /// * If opening the key or password file failed
    /// * If reading the key or password failed
    /// * If splitting the key or password command failed
    /// * If executing the key or password command failed
    /// * If reading the key or password from the command failed
    ///
    /// # Returns
    ///
    /// The credentials or `None`
    pub fn credentials(&self) -> RusticResult<Option<Credentials>> {
        // helpers
        fn get_key(key: impl AsRef<[u8]>) -> RusticResult<MasterKey> {
            let key = key.as_ref();
            serde_json::from_slice(key).map_err(|err| {
                RusticError::with_source(ErrorKind::Credentials, "Error deserializing key", err)
            })
        }
        let read_password_file = |file: &Path| {
            let mut file = BufReader::new(File::open(file).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Credentials,
                    "Opening password file failed. Is the path `{path}` correct?",
                    err,
                )
                .attach_context("path", file.display().to_string())
            })?);

            read_password_from_reader(&mut file)
        };
        let read_key_file = |file: &Path| {
            std::fs::read_to_string(file).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Credentials,
                    "Opening key file failed. Is the path `{path}` correct?",
                    err,
                )
                .attach_context("path", file.display().to_string())
            })
        };
        let pass_from_command = |command: &CommandInput| {
            let output = command.stdout()?;
            let mut pwd = BufReader::new(&*output);
            read_password_from_reader(&mut pwd)
        };

        let credentials = if let Some(key) = &self.key {
            Some(Credentials::Masterkey(get_key(key)?))
        } else if let Some(file) = &self.key_file {
            Some(Credentials::Masterkey(get_key(&read_key_file(file)?)?))
        } else if let Some(command) = &self.key_command {
            Some(Credentials::Masterkey(get_key(command.stdout()?)?))
        } else if let Some(pwd) = &self.password {
            Some(Credentials::Password(pwd.clone()))
        } else if let Some(file) = &self.password_file {
            Some(Credentials::Password(read_password_file(file)?))
        } else if let Some(command) = &self.password_command {
            Some(Credentials::Password(pass_from_command(command)?))
        } else {
            None
        };
        Ok(credentials)
    }
}

/// Read a password from a reader
///
/// # Arguments
///
/// * `file` - The reader to read the password from
///
/// # Errors
///
/// * If reading the password failed
pub fn read_password_from_reader(file: &mut impl BufRead) -> RusticResult<String> {
    let mut password = String::new();
    _ = file.read_line(&mut password).map_err(|err| {
        RusticError::with_source(
            ErrorKind::Credentials,
            "Reading password from reader failed. Is the file empty? Please check the file and the password.",
            err
        )
        .attach_context("password", password.clone())
    })?;

    // Remove the \n from the line if present
    if password.ends_with('\n') {
        _ = password.pop();
    }

    // Remove the \r from the line if present
    if password.ends_with('\r') {
        _ = password.pop();
    }

    Ok(password)
}
