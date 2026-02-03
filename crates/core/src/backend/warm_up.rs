use std::sync::Arc;

use bytes::Bytes;

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    error::RusticResult,
    id::Id,
};

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
    pub fn new_warm_up(be: Arc<dyn WriteBackend>) -> Arc<dyn WriteBackend> {
        Arc::new(Self { be })
    }
}

impl ReadBackend for WarmUpAccessBackend {
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

    fn needs_warm_up(&self) -> bool {
        true
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        // warm up files by accessing them - error is ignored as we expect this to error out!
        _ = self.be.read_partial(tpe, id, false, 0, 1);
        Ok(())
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        // Delegate to the underlying backend
        self.be.warmup_path(tpe, id)
    }
}

impl WriteBackend for WarmUpAccessBackend {
    fn create(&self) -> RusticResult<()> {
        self.be.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        // First remove cold file
        self.be.remove(tpe, id, cacheable)
    }
}
