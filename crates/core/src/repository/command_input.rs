use std::{
    fmt::{Debug, Display},
    process::{Command, ExitStatus},
    str::FromStr,
};

use log::{debug, error, trace, warn};
use serde::{Deserialize, Serialize, Serializer};
use serde_with::{serde_as, DisplayFromStr, PickFirst};

use crate::{
    error::{RepositoryErrorKind, RusticErrorKind},
    RusticError, RusticResult,
};

/// A command to be called which can be given as CLI option as well as in config files
/// `CommandInput` implements Serialize/Deserialize as well as FromStr.
#[serde_as]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct CommandInput(
    // Note: we use _CommandInput here which itself impls FromStr in order to use serde_as PickFirst for CommandInput.
    //#[serde(
    //    serialize_with = "serialize_command",
    //    deserialize_with = "deserialize_command"
    //)]
    #[serde_as(as = "PickFirst<(DisplayFromStr,_)>")] _CommandInput,
);

impl Serialize for CommandInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // if on_failure is default, we serialize to the short `Display` version, else we serialize the struct
        if self.0.on_failure == OnFailure::default() {
            serializer.serialize_str(&self.to_string())
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl From<Vec<String>> for CommandInput {
    fn from(value: Vec<String>) -> Self {
        Self(value.into())
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
        !self.0.command.is_empty()
    }

    /// Returns the command
    #[must_use]
    pub fn command(&self) -> &str {
        &self.0.command
    }

    /// Returns the command args
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.0.args
    }

    /// Returns the error handling for the command
    #[must_use]
    pub fn on_failure(&self) -> &OnFailure {
        &self.0.on_failure
    }

    /// Runs the command if it is set
    ///
    /// # Errors
    ///
    /// `RusticError` if return status cannot be read
    pub fn run(&self, context: &str, what: &str) -> RusticResult<()> {
        if !self.is_set() {
            trace!("not calling command {context}:{what} - not set");
            return Ok(());
        }
        debug!("calling command {context}:{what}: {self:?}");
        let status = Command::new(self.command()).args(self.args()).status();
        self.on_failure().handle_status(status, context, what)?;
        Ok(())
    }
}

impl FromStr for CommandInput {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(_CommandInput::from_str(s)?))
    }
}

impl Display for CommandInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct _CommandInput {
    command: String,
    args: Vec<String>,
    on_failure: OnFailure,
}

impl _CommandInput {
    fn iter(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.command).chain(self.args.iter())
    }
}

impl From<Vec<String>> for _CommandInput {
    fn from(mut value: Vec<String>) -> Self {
        if value.is_empty() {
            Self::default()
        } else {
            let command = value.remove(0);
            Self {
                command,
                args: value,
                ..Default::default()
            }
        }
    }
}

impl FromStr for _CommandInput {
    type Err = RusticError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(split(s)?.into())
    }
}

impl Display for _CommandInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = shell_words::join(self.iter());
        f.write_str(&s)
    }
}

/// Error handling for commands called as `CommandInput`
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnFailure {
    /// errors in command calling will result in rustic errors
    #[default]
    Error,
    /// errors in command calling will result in rustic warnings, but are otherwise ignored
    Warn,
    /// errors in command calling will be ignored
    Ignore,
}

impl OnFailure {
    fn eval<T>(&self, res: RusticResult<T>) -> RusticResult<Option<T>> {
        match res {
            Err(err) => match self {
                OnFailure::Error => {
                    error!("{err}");
                    Err(err)
                }
                OnFailure::Warn => {
                    warn!("{err}");
                    Ok(None)
                }
                OnFailure::Ignore => Ok(None),
            },
            Ok(res) => Ok(Some(res)),
        }
    }

    /// Handle a status of a called command depending on the defined error handling
    pub fn handle_status(
        &self,
        status: Result<ExitStatus, std::io::Error>,
        context: &str,
        what: &str,
    ) -> RusticResult<()> {
        let status = status.map_err(|err| {
            RepositoryErrorKind::CommandExecutionFailed(context.into(), what.into(), err).into()
        });
        let Some(status) = self.eval(status)? else {
            return Ok(());
        };

        if !status.success() {
            let _: Option<()> = self.eval(Err(RepositoryErrorKind::CommandErrorStatus(
                context.into(),
                what.into(),
                status,
            )
            .into()))?;
        }
        Ok(())
    }
}

/// helper to split arguments
// TODO: Maybe use special parser (winsplit?) for windows?
fn split(s: &str) -> RusticResult<Vec<String>> {
    Ok(shell_words::split(s).map_err(|err| RusticErrorKind::Command(err.into()))?)
}
