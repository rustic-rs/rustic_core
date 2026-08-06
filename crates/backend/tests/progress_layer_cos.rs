// crates/backend/tests/progress_layer_cos.rs
#![cfg(feature = "opendal")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rustic_backend::OpenDALBackend;
use rustic_core::{FileType, Id, WriteBackend};

/// Per-part granularity verification against a real COS backend.
///
/// This test is `#[ignore]` by default; it only runs when `--ignored` is explicitly passed,
/// and it requires the following environment variables:
///   COS_SECRET_ID   —— Tencent Cloud SecretId
///   COS_SECRET_KEY  —— Tencent Cloud SecretKey
///   COS_BUCKET      —— bucket name (COS usually needs the appid suffix, e.g. my-bucket-1250000000)
///   COS_ENDPOINT    —— region/endpoint (per the opendal 0.57 services-cos docs you use)
///   COS_ROOT        —— optional, repository root path
///
/// If any required variable is missing, it skips immediately (returns Ok(())), ensuring
/// environments without credentials (including the official CI) will not fail.
///
/// How to run:
///   cargo test -p rustic_backend --features opendal --test progress_layer_cos -- --ignored --nocapture
///
/// Interpretation: if the counter increases in multiple steps (e.g. 8MiB -> 16MiB -> 24MiB) then ...
/// per-part granularity is in effect; if it jumps to 16MiB instantly and then write_bytes returns
/// only much later, that opendal version / COS service is buffering the whole payload inside the
/// writer, and you need to tune the writer's chunk/buffer size to align with the part size
/// (the adjustment point is in the opendal writer configuration, not the counting layer itself).
#[test]
#[ignore]
fn cos_progress_layer_per_part_granularity() -> Result<()> {
    let _ = dotenvy::dotenv();
    // 1) Read credentials from environment variables; skip if any required item is missing.
    let (secret_id, secret_key, bucket, endpoint) = match (
        std::env::var("COS_SECRET_ID"),
        std::env::var("COS_SECRET_KEY"),
        std::env::var("COS_BUCKET"),
        std::env::var("COS_ENDPOINT"),
    ) {
        (Ok(id), Ok(key), Ok(bucket), Ok(endpoint)) => (id, key, bucket, endpoint),
        _ => {
            eprintln!("COS credentials not set, skipping cos_progress_layer_per_part_granularity");
            return Ok(());
        }
    };

    // 2) Build options. The key names for opendal 0.57 services-cos follow the actual docs.
    let mut options: BTreeMap<String, String> = BTreeMap::new();
    let _ = options.insert("secret_id".to_string(), secret_id);
    let _ = options.insert("secret_key".to_string(), secret_key);
    let _ = options.insert("bucket".to_string(), bucket);
    let _ = options.insert("endpoint".to_string(), endpoint);

    // Optional root: use if let to avoid producing a discarded Option<()>.
    if let Ok(root) = std::env::var("COS_ROOT") {
        let _ = options.insert("root".to_string(), root);
    }

    // 3) Shared counter + backend with the counting layer.
    let counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let be = OpenDALBackend::new_with_progress("cos", options, Some(counter.clone()))?;

    // 4) Build pack data spanning >2 multipart parts (128 MiB; S3/COS min part is ~8 MiB).
    let data = vec![0u8; 128 * 1024 * 1024];
    let id = Id::random();

    // 5) Background thread periodically prints the counter to observe whether it increases per part flush.
    let stop = Arc::new(AtomicBool::new(false));
    let monitor = {
        let counter = counter.clone();
        let stop = stop.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let written = counter.load(Ordering::Relaxed);
                eprintln!("PROGRESS-COS t={ts} counter={written}");
                thread::sleep(Duration::from_millis(200));
            }
        })
    };

    // 6) Perform the upload (blocks until the whole payload is written).
    let write_result = be.write_bytes(FileType::Pack, &id, false, data.clone().into());

    // 7) Stop the monitor thread and wait for it to exit.
    stop.store(true, Ordering::Relaxed);
    let _ = monitor.join();

    // 8) Regardless of whether the write succeeded, attempt cleanup to avoid leaving junk objects in the real bucket.
    let _ = be.remove(FileType::Pack, &id, false);

    // 9) The write must succeed, and the count must cover at least the logical byte size.
    write_result?;
    let written = counter.load(Ordering::Relaxed);
    assert!(
        written >= data.len() as u64,
        "counter should be >= written bytes; got {written}, expected >= {}",
        data.len()
    );

    Ok(())
}