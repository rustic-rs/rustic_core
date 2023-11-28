use std::sync::Arc;

use bytes::Bytes;

use crate::{backend::FileType, backend::ReadBackend, backend::WriteBackend, id::Id, RusticResult};

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
    pub fn new(
        be: Arc<dyn WriteBackend>,
        be_hot: Option<Arc<dyn WriteBackend>>,
    ) -> Arc<dyn WriteBackend> {
        if let Some(be_hot) = be_hot {
            Arc::new(Self { be, be_hot })
        } else {
            be
        }
    }
}

impl ReadBackend for HotColdBackend {
    fn location(&self) -> String {
        self.be.location()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.be_hot.read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        if cacheable || tpe != FileType::Pack {
            self.be_hot.read_partial(tpe, id, cacheable, offset, length)
        } else {
            self.be.read_partial(tpe, id, cacheable, offset, length)
        }
    }

    fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up()
    }

    fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
        self.be.warm_up(tpe, id)
    }
}

impl WriteBackend for HotColdBackend {
    fn create(&self) -> RusticResult<()> {
        self.be.create()?;
        self.be_hot.create()
    }

    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        if tpe != FileType::Config && (cacheable || tpe != FileType::Pack) {
            self.be_hot.write_bytes(tpe, id, cacheable, buf.clone())?;
        }
        self.be.write_bytes(tpe, id, cacheable, buf)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        // First remove cold file
        self.be.remove(tpe, id, cacheable)?;
        if cacheable || tpe != FileType::Pack {
            self.be_hot.remove(tpe, id, cacheable)?;
        }
        Ok(())
    }
}
