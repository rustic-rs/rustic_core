use std::{
    fmt::{Debug, Display},
    process::{Command, ExitStatus},
    str::FromStr,
};

use log::{debug, error, trace, warn};
use serde::{Deserialize, Serialize, Serializer};
use serde_with::{serde_as, DisplayFromStr, PickFirst};

use crate::error::RusticResult;

/// [`CommandInputErrorKind`] describes the errors that can be returned from the `CommandInput`
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum CommandInputErrorKind {
    /// Command execution failed: {context}:{what} : {source}
    CommandExecutionFailed {
        /// The context in which the command was called
        context: String,

        /// The action that was performed
        what: String,

        /// The source of the error
        source: std::io::Error,
    },
    /// Command error status: {context}:{what} : {status}
    CommandErrorStatus {
        /// The context in which the command was called
        context: String,

        /// The action that was performed
        what: String,

        /// The exit status of the command
        status: ExitStatus,
    },
    /// Splitting arguments failed: {arguments} : {source}
    SplittingArgumentsFailed {
        /// The arguments that were tried to be split
        arguments: String,

        /// The source of the error
        source: shell_words::ParseError,
    },
    /// Process execution failed: {command:?} : {path:?} : {source}
    ProcessExecutionFailed {
        /// The command that was tried to be executed
        command: CommandInput,

        /// The path in which the command was tried to be executed
        path: std::path::PathBuf,

        /// The source of the error
        source: std::io::Error,
    },
}

pub(crate) type CommandInputResult<T> = Result<T, CommandInputErrorKind>;

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
    pub fn on_failure(&self) -> OnFailure {
        self.0.on_failure
    }

    /// Runs the command if it is set
    ///
    /// # Errors
    ///
    /// `CommandInputErrorKind` if return status cannot be read
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
    type Err = CommandInputErrorKind;
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
    type Err = CommandInputErrorKind;
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
    fn eval<T>(self, res: CommandInputResult<T>) -> RusticResult<Option<T>> {
        let res = self.display_result(res);
        match (res, self) {
            (Err(err), Self::Error) => Err(err),
            (Err(_), _) => Ok(None),
            (Ok(res), _) => Ok(Some(res)),
        }
    }

    /// Displays a result depending on the defined error handling which still yielding the same result
    ///
    /// # Note
    ///
    /// This can be used where an error might occur, but in that
    /// case we have to abort.
    pub fn display_result<T>(self, res: CommandInputResult<T>) -> RusticResult<T> {
        if let Err(err) = &res {
            match self {
                Self::Error => {
                    error!("{err}");
                }
                Self::Warn => {
                    warn!("{err}");
                }
                Self::Ignore => {}
            }
        }
        res.map_err(|_err| todo!("Error transition"))
    }

    /// Handle a status of a called command depending on the defined error handling
    pub fn handle_status(
        self,
        status: Result<ExitStatus, std::io::Error>,
        context: &str,
        what: &str,
    ) -> RusticResult<()> {
        let status = status.map_err(|err| CommandInputErrorKind::CommandExecutionFailed {
            context: context.to_string(),
            what: what.to_string(),
            source: err,
        });

        let Some(status) = self.eval(status)? else {
            return Ok(());
        };

        if !status.success() {
            let _: Option<()> = self.eval(Err(CommandInputErrorKind::CommandErrorStatus {
                context: context.to_string(),
                what: what.to_string(),
                status,
            }))?;
        }
        Ok(())
    }
}

/// helper to split arguments
// TODO: Maybe use special parser (winsplit?) for windows?
fn split(s: &str) -> CommandInputResult<Vec<String>> {
    shell_words::split(s).map_err(|err| CommandInputErrorKind::SplittingArgumentsFailed {
        arguments: s.to_string(),
        source: err,
    })
}
