use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;

use crate::{
    backend::{FileType, ReadBackend, WriteBackend},
    id::Id,
};

use super::{AsyncReadBackend, AsyncWriteBackend};

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

#[derive(Clone, Debug)]
pub struct AsyncHotColdBackend {
    /// The backend to use.
    be: Arc<dyn AsyncWriteBackend>,
    /// The backend to use for hot files.
    be_hot: Arc<dyn AsyncWriteBackend>,
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
    pub fn new<BE: WriteBackend>(be: BE, hot_be: BE) -> Self {
        Self {
            be: Arc::new(be),
            be_hot: Arc::new(hot_be),
        }
    }
}

impl AsyncHotColdBackend {
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
    pub fn new<BE: AsyncWriteBackend>(be: BE, hot_be: BE) -> Self {
        Self {
            be: Arc::new(be),
            be_hot: Arc::new(hot_be),
        }
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

#[async_trait]
impl AsyncReadBackend for AsyncHotColdBackend {
    fn location(&self) -> String {
        self.be.location()
    }

    async fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
        self.be.list_with_size(tpe).await
    }

    async fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
        self.be_hot.read_full(tpe, id).await
    }

    async fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> Result<Bytes> {
        if cacheable || tpe != FileType::Pack {
            self.be_hot
                .read_partial(tpe, id, cacheable, offset, length)
                .await
        } else {
            self.be
                .read_partial(tpe, id, cacheable, offset, length)
                .await
        }
    }

    async fn needs_warm_up(&self) -> bool {
        self.be.needs_warm_up().await
    }

    async fn warm_up(&self, tpe: FileType, id: &Id) -> Result<()> {
        self.be.warm_up(tpe, id).await
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

#[async_trait]
impl AsyncWriteBackend for AsyncHotColdBackend {
    async fn create(&self) -> Result<()> {
        self.be.create().await?;
        self.be_hot.create().await
    }

    async fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> Result<()> {
        if tpe != FileType::Config && (cacheable || tpe != FileType::Pack) {
            self.be_hot
                .write_bytes(tpe, id, cacheable, buf.clone())
                .await?;
        }
        self.be.write_bytes(tpe, id, cacheable, buf).await
    }

    async fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> Result<()> {
        // First remove cold file
        self.be.remove(tpe, id, cacheable).await?;
        if cacheable || tpe != FileType::Pack {
            self.be_hot.remove(tpe, id, cacheable).await?;
        }
        Ok(())
    }
}
