use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;

use crate::{backend::FileType, backend::ReadBackend, backend::WriteBackend, id::Id};

/// A backend which warms up files by simply accessing them.
#[derive(Clone, Debug)]
pub struct WarmUpAccessBackend {
    /// The backend to use.
    be: Arc<dyn WriteBackend>,
}

impl WarmUpAccessBackend {
    /// Creates a new `WarmUpAccessBackend`.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use.
    pub fn new(be: Arc<dyn WriteBackend>) -> Arc<dyn WriteBackend> {
        Arc::new(Self { be })
    }
}

impl ReadBackend for WarmUpAccessBackend {
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

    fn needs_warm_up(&self) -> bool {
        true
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> Result<()> {
        // warm up files by accessing them - error is ignored as we expect this to error out!
        _ = self.be.read_partial(tpe, id, false, 0, 1);
        Ok(())
    }
}

impl WriteBackend for WarmUpAccessBackend {
    fn create(&self) -> Result<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        // First remove cold file
        self.be.remove(tpe, id, cacheable)
    }
}
