use std::{fmt::Display, process::Command, str::FromStr};

use log::{debug, trace, warn};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, PickFirst};

use crate::{error::RusticErrorKind, RusticError, RusticResult};

/// A command to be called which can be given as CLI option as well as in config files
/// `CommandInput` implements Serialize/Deserialize as well as FromStr.
#[serde_as]
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInput(
    // Note: we use CommandInputInternal here which itself impls FromStr in order to use serde_as PickFirst for CommandInput.
    #[serde_as(as = "PickFirst<(DisplayFromStr,_)>")] CommandInputInternal,
);

impl From<Vec<String>> for CommandInput {
    fn from(value: Vec<String>) -> Self {
        Self(CommandInputInternal::from_vec(value))
    }
}

impl From<CommandInput> for Vec<String> {
    fn from(value: CommandInput) -> Self {
        value.0.iter().cloned().collect()
    }
}

impl CommandInput {
    /// Returns if a command is set
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.0.command.is_some()
    }

    /// Returns the command if it is set
    ///
    /// # Panics
    ///
    /// Panics if no command is set.
    #[must_use]
    pub fn command(&self) -> &str {
        self.0.command.as_ref().unwrap()
    }

    /// Returns the command args if it is set
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.0.args.0
    }

    /// Runs the command if it is set
    ///
    /// # Errors
    ///
    /// `std::io::Error` if return status cannot be read
    pub fn run(&self, context: &str, what: &str) -> Result<(), std::io::Error> {
        if !self.is_set() {
            trace!("not calling command {context}:{what} - not set");
            return Ok(());
        }
        debug!("calling command {context}:{what}: {self:?}");
        let status = Command::new(self.command()).args(self.args()).status()?;
        if !status.success() {
            warn!("running command {context}:{what} was not successful. {status}");
        }
        Ok(())
    }
}

impl FromStr for CommandInput {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(CommandInputInternal::from_str(s)?))
    }
}

impl Display for CommandInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[serde_as]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct CommandInputInternal {
    command: Option<String>,
    #[serde_as(as = "PickFirst<(DisplayFromStr,_)>")]
    args: ArgInternal,
}

impl CommandInputInternal {
    fn iter(&self) -> impl Iterator<Item = &String> {
        self.command.iter().chain(self.args.0.iter())
    }

    fn from_vec(mut vec: Vec<String>) -> Self {
        if vec.is_empty() {
            Self::default()
        } else {
            let command = Some(vec.remove(0));
            Self {
                command,
                args: ArgInternal(vec),
            }
        }
    }
}

impl FromStr for CommandInputInternal {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_vec(split(s)?))
    }
}

impl Display for CommandInputInternal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = shell_words::join(self.iter());
        f.write_str(&s)
    }
}

#[serde_as]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
struct ArgInternal(Vec<String>);

impl FromStr for ArgInternal {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(split(s)?))
    }
}

// helper to split arguments
// TODO: Maybe use special parser (winsplit?) for windows?
fn split(s: &str) -> RusticResult<Vec<String>> {
    Ok(shell_words::split(s).map_err(|err| RusticErrorKind::Command(err.into()))?)
}
