// crates/backend/tests/progress_layer_webdav.rs
#![cfg(feature = "opendal")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use rustic_backend::OpenDALBackend;
use rustic_core::{FileType, Id, WriteBackend};

/// OneShot fallback verification against a real WebDAV backend.
///
/// opendal's WebDAV service only provides a `OneShotWriter` and does not
/// support multiple `write` calls. Before the `write_can_multi` guard was
/// added in `write_bytes`, any pack larger than `CHUNK_SIZE` (8 MiB) failed
/// with `OneShotWriter doesn't support multiple write` while smaller files
/// passed by luck (single chunk). This test writes a 24 MiB payload — which
/// spans multiple chunks — to prove the fallback to a single `operator.write`
/// succeeds and progress is still counted.
///
/// This test is `#[ignore]` by default; it only runs when `--ignored` is
/// explicitly passed, and it requires the following environment variables:
///   WEBDAV_ENDPOINT  —— e.g. http://192.168.199.252:48080
///   WEBDAV_USERNAME  —— WebDAV login user
///   WEBDAV_PASSWORD  —— WebDAV login password
///   WEBDAV_ROOT      —— optional, repository root path (e.g. /webdav)
///
/// If any required variable is missing, it skips immediately (returns Ok(())),
/// ensuring environments without credentials (including the official CI) will
/// not fail.
///
/// How to run:
///   cargo test -p rustic_backend --features opendal --test progress_layer_webdav -- --ignored --nocapture
#[test]
#[ignore]
fn webdav_oneshot_fallback_writes_large_pack() -> Result<()> {
    let _ = dotenvy::dotenv();

    // 1) Read credentials from environment variables; skip if any required item is missing.
    let (endpoint, username, password) = match (
        std::env::var("WEBDAV_ENDPOINT"),
        std::env::var("WEBDAV_USERNAME"),
        std::env::var("WEBDAV_PASSWORD"),
    ) {
        (Ok(endpoint), Ok(username), Ok(password)) => (endpoint, username, password),
        _ => {
            eprintln!(
                "WebDAV credentials not set, skipping webdav_oneshot_fallback_writes_large_pack"
            );
            return Ok(());
        }
    };

    // 2) Build options. Key names follow opendal 0.57 services-webdav.
    let mut options: BTreeMap<String, String> = BTreeMap::new();
    let _ = options.insert("endpoint".to_string(), endpoint);
    let _ = options.insert("username".to_string(), username);
    let _ = options.insert("password".to_string(), password);
    if let Ok(root) = std::env::var("WEBDAV_ROOT") {
        let _ = options.insert("root".to_string(), root);
    }

    // 3) Shared counter + backend with the counting layer.
    let counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let be = OpenDALBackend::new_with_progress("webdav", options, Some(counter.clone()))?;

    // 4) 24 MiB payload: larger than CHUNK_SIZE (8 MiB), i.e. the exact case that
    //    previously triggered "OneShotWriter doesn't support multiple write".
    let data = vec![0u8; 24 * 1024 * 1024];
    let id = Id::random();

    // 5) Single-shot write on OneShot backends: no per-part monitor thread needed,
    //    the counter jumps once when the whole payload is flushed.
    let write_result = be.write_bytes(FileType::Pack, &id, false, data.clone().into());

    // 6) Cleanup regardless of outcome to avoid leaving junk on the real server.
    let _ = be.remove(FileType::Pack, &id, false);

    // 7) The write must succeed (no OneShotWriter error), and the counter must
    //    cover at least the logical byte size.
    write_result?;
    let written = counter.load(Ordering::Relaxed);
    assert!(
        written >= data.len() as u64,
        "counter should be >= written bytes; got {written}, expected >= {}",
        data.len()
    );

    Ok(())
}