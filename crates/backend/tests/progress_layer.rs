// crates/backend/tests/progress_layer.rs
#![cfg(feature = "opendal")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use rustic_backend::OpenDALBackend;
use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

/// Counting path connectivity: after writing through a backend with a counter, the counter should be >= the number of bytes written.
#[test]
fn progress_layer_counts_written_bytes() -> Result<()> {
    // 1) Shared counter
    let counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

    // 2) Build a backend with the counting layer using the memory service
    let be = OpenDALBackend::new_with_progress(
        "memory",
        BTreeMap::new(),
        Some(counter.clone()),
    )?;

    // 3) Write a chunk of data
    let data = vec![0u8; 4096];
    be.write_bytes(FileType::Pack, &Id::random(), false, data.clone().into())?;

    // 4) Assert the counting path was triggered (memory does not use multipart, may complete in one shot)
    let written = counter.load(Ordering::Relaxed);
    assert!(
        written >= data.len() as u64,
        "counter should be >= written bytes; got {written}, expected >= {}",
        data.len()
    );

    Ok(())
}

/// Default path regression: with counter=None the write behaves normally and does not panic.
#[test]
fn default_path_without_counter_still_writes() -> Result<()> {
    let be = OpenDALBackend::new("memory", BTreeMap::new())?;

    let data = vec![1u8; 4096];
    let id = Id::random();
    be.write_bytes(FileType::Pack, &id, false, data.clone().into())?;

    // Read back and verify content matches (default read/write path works)
    let read_back = be.read_full(FileType::Pack, &id)?;
    assert_eq!(read_back.as_ref(), data.as_slice());

    Ok(())
}