//! Backend with attached concurrency information

use bytes::Bytes;
use std::ops::Deref;
use std::sync::Arc;

use crate::{
    backend::{BytesList, FileType, ReadBackend, WriteBackend},
    concurrency::{ConcurrencyClass, ConcurrencyManager},
    error::RusticResult,
    id::Id,
};

/// A backend with attached concurrency information
#[derive(Clone, Debug)]
pub struct ConcurrencyBackend<BE: WriteBackend> {
    pub be: BE,
    concurrency: ConcurrencyManager,
}

/// Trait for backends with concurrency information
pub trait ConcurrentBackend {
    fn concurrency(&self) -> &ConcurrencyManager;
}

pub trait ConcurrentReadBackend: ConcurrentBackend + ReadBackend {}
pub trait ConcurrentWriteBackend: ConcurrentReadBackend + WriteBackend {}

// Implement backend traits for existing types
impl<T: ConcurrentBackend + ReadBackend> ConcurrentReadBackend for T {}
impl<T: ConcurrentBackend + WriteBackend> ConcurrentWriteBackend for T {}

impl<BE: WriteBackend> ConcurrencyBackend<BE> {
    /// Create a new [`ConcurrencyBackend`]
    pub fn new(be: BE, concurrency: ConcurrencyManager) -> Self {
        Self { be, concurrency }
    }

    pub fn concurrency(&self) -> &ConcurrencyManager {
        &self.concurrency
    }
}

//
// Backend trait implementations (Concurrent, Read and Write)
//
impl<BE: WriteBackend> ConcurrentBackend for ConcurrencyBackend<BE> {
    fn concurrency(&self) -> &ConcurrencyManager {
        self.concurrency()
    }
}

impl<BE: WriteBackend> ReadBackend for ConcurrencyBackend<BE> {
    fn location(&self) -> String {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.location()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.list_with_size(tpe)
    }

    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.list(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
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
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.read_partial(tpe, id, cacheable, offset, length)
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.warmup_path(tpe, id)
    }

    fn needs_warm_up(&self) -> bool {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.needs_warm_up()
    }

    fn warm_up(&self, _tpe: FileType, _id: &Id) -> RusticResult<()> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.warm_up(_tpe, _id)
    }
}

impl<BE: WriteBackend> WriteBackend for ConcurrencyBackend<BE> {
    fn create(&self) -> RusticResult<()> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.create()
    }

    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        content: BytesList,
    ) -> RusticResult<()> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.write_bytes(tpe, id, cacheable, content)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        let _guard = self.concurrency().acquire(ConcurrencyClass::Backend);
        self.be.remove(tpe, id, cacheable)
    }
}

//
// Backend trait implementations for trait objects in a Repository
//

impl ReadBackend for Arc<dyn ConcurrentWriteBackend> {
    fn location(&self) -> String {
        self.deref().location()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.deref().list_with_size(tpe)
    }

    fn list(&self, tpe: FileType) -> RusticResult<Vec<Id>> {
        self.deref().list(tpe)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.deref().read_full(tpe, id)
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        self.deref()
            .read_partial(tpe, id, cacheable, offset, length)
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.deref().warmup_path(tpe, id)
    }

    fn needs_warm_up(&self) -> bool {
        self.deref().needs_warm_up()
    }

    fn warm_up(&self, _tpe: FileType, _id: &Id) -> RusticResult<()> {
        self.deref().warm_up(_tpe, _id)
    }
}

impl WriteBackend for Arc<dyn ConcurrentWriteBackend> {
    fn create(&self) -> RusticResult<()> {
        self.deref().create()
    }

    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        content: BytesList,
    ) -> RusticResult<()> {
        self.deref().write_bytes(tpe, id, cacheable, content)
    }

    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        self.deref().remove(tpe, id, cacheable)
    }
}

impl ConcurrentBackend for Arc<dyn ConcurrentWriteBackend> {
    fn concurrency(&self) -> &ConcurrencyManager {
        self.deref().concurrency()
    }
}

impl std::fmt::Debug for dyn ConcurrentWriteBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConcurrentWriteBackend{{{}}}", self.location())
    }
}
