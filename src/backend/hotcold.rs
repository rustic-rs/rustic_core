use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;

use crate::{backend::FileType, backend::ReadBackend, backend::WriteBackend, id::Id};

/// A hot/cold backend implementation.
///
/// # Type Parameters
///
/// * `BE` - The backend to use.
#[derive(Clone, Debug)]
pub struct HotColdBackend {
    /// The backend to use.
    be: Arc<dyn WriteBackend>,
    /// The backend to use for hot files.
    be_hot: Arc<dyn WriteBackend>,
}

impl HotColdBackend {
    /// Creates a new `HotColdBackend`.
    ///
    /// # Type Parameters
    ///
    /// * `BE` - The backend to use.
    ///
    /// # Arguments
    ///
    /// * `be` - The backend to use.
    /// * `hot_be` - The backend to use for hot files.
    pub fn new(be: Arc<dyn WriteBackend>, be_hot: Arc<dyn WriteBackend>) -> Arc<dyn WriteBackend> {
        Arc::new(Self { be, be_hot })
    }
}

impl ReadBackend for HotColdBackend {
    fn location(&self) -> String {
        self.be.location()
    }

    fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.be_hot.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        if cacheable || tpe != FileType::Pack {
            self.be_hot.read_partial(tpe, id, cacheable, offset, length)
        } else {
            self.be.read_partial(tpe, id, cacheable, offset, length)
        }
    }

    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> Result<()> {
        self.be.warm_up(tpe, id)
    }
}

impl WriteBackend for HotColdBackend {
    fn create(&self) -> Result<()> {
        self.be.create()?;
        self.be_hot.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        if tpe != FileType::Config && (cacheable || tpe != FileType::Pack) {
            self.be_hot.write_bytes(tpe, id, cacheable, buf.clone())?;
        }
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        // First remove cold file
        self.be.remove(tpe, id, cacheable)?;
        if cacheable || tpe != FileType::Pack {
            self.be_hot.remove(tpe, id, cacheable)?;
        }
        Ok(())
    }
}
