use std::{process::Command, sync::Arc};

use anyhow::Result;
use bytes::Bytes;
use chrono::{DateTime, Local};
use log::{debug, warn};

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    id::Id,
    CommandInput,
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

    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.be.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        self.be.read_partial(tpe, id, cacheable, offset, length)
    }

    fn list(&self, tpe: FileType) -> Result<Vec<Id>> {
        self.be.list(tpe)
    }

    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> Result<()> {
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
    fn create(&self) -> Result<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        self.be.remove(tpe, id, cacheable)
    }

    fn can_lock(&self) -> bool {
        true
    }

    fn lock(&self, tpe: FileType, id: &Id, until: Option<DateTime<Local>>) -> Result<()> {
        let until = until.map_or_else(String::new, |u| u.to_rfc3339().to_string());
        let path = path(tpe, id);
        let args = self.command.args().iter().map(|c| {
            c.replace("%id", &id.to_hex())
                .replace("%type", tpe.dirname())
                .replace("%path", &path)
                .replace("%until", &until)
        });
        debug!("calling {:?}...", self.command);
        let status = Command::new(self.command.command()).args(args).status()?;
        if !status.success() {
            warn!("lock command was not successful for {tpe:?}, id: {id}. {status}");
        }
        Ok(())
    }
}
