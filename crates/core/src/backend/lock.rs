use std::{process::Command, sync::Arc};

use bytes::Bytes;
use chrono::{DateTime, Local};
use log::{debug, warn};

use crate::{
    CommandInput, ErrorKind, RusticError, RusticResult,
    backend::{FileType, ReadBackend, WriteBackend},
    id::Id,
};

/// A backend which warms up files by simply accessing them.
#[derive(Clone, Debug)]
pub struct LockBackend {
    /// The backend to use.
    be: Arc<dyn WriteBackend>,
    /// The command to be called to lock files in the backend
    command: CommandInput,
}

impl LockBackend {
    /// Creates a new `WarmUpAccessBackend`.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use.
    pub fn new_lock(be: Arc<dyn WriteBackend>, command: CommandInput) -> Arc<dyn WriteBackend> {
        Arc::new(Self { be, command })
    }
}

impl ReadBackend for LockBackend {
    fn location(&self) -> String {
        self.be.location()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.be.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        self.be.read_partial(tpe, id, cacheable, offset, length)
    }

    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
        self.be.list(tpe)
    }

    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        self.be.warm_up(tpe, id)
    }
}

fn path(tpe: FileType, id: &Id) -> String {
    let hex_id = id.to_hex();
    match tpe {
        FileType::Config => "config".into(),
        FileType::Pack => format!("data/{}/{}", &hex_id[0..2], &*hex_id),
        _ => format!("{}/{}", tpe.dirname(), &*hex_id),
    }
}

impl WriteBackend for LockBackend {
    fn create(&self) -> RusticResult<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        self.be.remove(tpe, id, cacheable)
    }

    fn can_lock(&self) -> bool {
        true
    }

    fn lock(&self, tpe: FileType, id: &Id, until: Option<DateTime<Local>>) -> RusticResult<()> {
        let until = until.map_or_else(String::new, |u| u.to_rfc3339());
        let path = path(tpe, id);
        let args = self.command.args().iter().map(|c| {
            c.replace("%id", &id.to_hex())
                .replace("%type", tpe.dirname())
                .replace("%path", &path)
                .replace("%until", &until)
        });
        debug!("calling {:?}...", self.command);
        let status = Command::new(self.command.command())
            .args(args)
            .status()
            .map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "error calling lock command for {tpe}, id: {id}.",
                    err,
                )
                .attach_context("tpe", tpe.to_string())
                .attach_context("id", id.to_string())
            })?;
        if !status.success() {
            warn!("lock command was not successful for {tpe:?}, id: {id}. {status}");
        }
        Ok(())
    }
}
