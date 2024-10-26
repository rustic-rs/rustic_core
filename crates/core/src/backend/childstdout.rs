use std::{
    iter::{once, Once},
    path::PathBuf,
    process::{Child, ChildStdout, Command, Stdio},
    sync::Mutex,
};

use crate::{
    backend::{ReadSource, ReadSourceEntry},
    error::{ErrorKind, RusticError, RusticResult},
    repository::command_input::{CommandInput, CommandInputErrorKind},
};

/// The `ChildStdoutSource` is a `ReadSource` when spawning a child process and reading its stdout
#[derive(Debug)]
pub struct ChildStdoutSource {
    /// The path of the stdin entry.
    path: PathBuf,
    /// The child process
    ///
    /// # Note
    ///
    /// This is in a Mutex as we want to take out `ChildStdout`
    /// in the `entries` method - but this method only gets a
    /// reference of self.
    process: Mutex<Child>,
    /// the command which is called
    command: CommandInput,
}

impl ChildStdoutSource {
    /// Creates a new `ChildSource`.
    pub fn new(cmd: &CommandInput, path: PathBuf) -> RusticResult<Self> {
        let process = Command::new(cmd.command())
            .args(cmd.args())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| CommandInputErrorKind::ProcessExecutionFailed {
                command: cmd.clone(),
                path: path.clone(),
                source: err,
            });

        let process = cmd.on_failure().display_result(process)?;

        Ok(Self {
            path,
            process: Mutex::new(process),
            command: cmd.clone(),
        })
    }

    /// Finishes the `ChildSource`
    pub fn finish(self) -> RusticResult<()> {
        let status = self.process.lock().unwrap().wait();
        self.command
            .on_failure()
            .handle_status(status, "stdin-command", "call")?;
        Ok(())
    }
}

impl ReadSource for ChildStdoutSource {
    type Open = ChildStdout;
    type Iter = Once<RusticResult<ReadSourceEntry<ChildStdout>>>;

    fn size(&self) -> RusticResult<Option<u64>> {
        Ok(None)
    }

    fn entries(&self) -> Self::Iter {
        let open = self.process.lock().unwrap().stdout.take();
        once(
            ReadSourceEntry::from_path(self.path.clone(), open).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Backend,
                    "Failed to create ReadSourceEntry from ChildStdout",
                    err,
                )
            }),
        )
    }
}
