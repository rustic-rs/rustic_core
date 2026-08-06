// crates/backend/src/progress_layer.rs
//! A custom OpenDAL Layer that accumulates the number of bytes written in real time as data is flushed down to the underlying service.
//!
//! This Layer operates on the async operator (assembled before the blocking wrapper). It intercepts the
//! writer returned by the underlying accessor and accumulates the count after each `write(Buffer)` is passed through to inner.
//! Granularity = each chunk of bytes the underlying service writer receives per call (for S3-like services this is usually each multipart part).

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use opendal::raw::oio;
use opendal::raw::{
    Access, Layer, LayeredAccess, OpList, OpRead, OpWrite, RpDelete, RpList, RpRead,
    RpWrite,
};
use opendal::{Buffer, Metadata, Result};

/// Shared handle for the written-bytes counter. Callers (e.g. the JNI layer) keep an `Arc` clone for polling.
pub type WrittenCounter = Arc<AtomicU64>;

/// The counting Layer itself, holding the shared counter.
#[derive(Clone, Debug)]
pub struct ProgressLayer {
    counter: WrittenCounter,
}

impl ProgressLayer {
    /// Create a Layer with the given counter.
    #[must_use]
    pub fn new(counter: WrittenCounter) -> Self {
        Self { counter }
    }
}

impl<A: Access> Layer<A> for ProgressLayer {
    type LayeredAccess = ProgressAccessor<A>;

    fn layer(&self, inner: A) -> Self::LayeredAccess {
        ProgressAccessor {
            inner,
            counter: self.counter.clone(),
        }
    }
}

/// Accessor that wraps the underlying accessor. Everything except `write` is passed through to inner.
#[derive(Debug)]
pub struct ProgressAccessor<A: Access> {
    inner: A,
    counter: WrittenCounter,
}

impl<A: Access> LayeredAccess for ProgressAccessor<A> {
    type Inner = A;
    type Reader = A::Reader;
    type Writer = ProgressWriter<A::Writer>;
    type Lister = A::Lister;
    type Deleter = A::Deleter;
    type Copier = A::Copier;

    fn inner(&self) -> &Self::Inner {
        &self.inner
    }

    async fn read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::Reader)> {
        // The read path is not counted; pass through directly.
        self.inner.read(path, args).await
    }

    async fn write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::Writer)> {
        // After obtaining the underlying writer, wrap it with ProgressWriter to inject the counter.
        let (rp, writer) = self.inner.write(path, args).await?;
        Ok((rp, ProgressWriter::new(writer, self.counter.clone())))
    }

    async fn delete(&self) -> Result<(RpDelete, Self::Deleter)> {
        self.inner.delete().await
    }

    async fn list(&self, path: &str, args: OpList) -> Result<(RpList, Self::Lister)> {
        self.inner.list(path, args).await
    }
}

/// Wraps the underlying writer and accumulates the count after each successful `write`.
pub struct ProgressWriter<W> {
    inner: W,
    counter: WrittenCounter,
}

impl<W> ProgressWriter<W> {
    fn new(inner: W, counter: WrittenCounter) -> Self {
        Self { inner, counter }
    }
}

impl<W: oio::Write> oio::Write for ProgressWriter<W> {
    async fn write(&mut self, bs: Buffer) -> Result<()> {
        // Take the length first (bs will be moved into inner.write afterwards).
        let len = bs.len() as u64;
        self.inner.write(bs).await?;
        // Only count on successful write to avoid inflation from failures/retries.
        let _ = self.counter.fetch_add(len, Ordering::Relaxed);
        Ok(())
    }

    async fn close(&mut self) -> Result<Metadata> {
        self.inner.close().await
    }

    async fn abort(&mut self) -> Result<()> {
        self.inner.abort().await
    }
}