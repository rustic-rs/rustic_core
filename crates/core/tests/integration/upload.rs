//! Exercises the parallel pack-file upload path in `blob::packer`.
//!
//! The writer actor uploads several packs concurrently. These tests force many
//! small packs (far more than the upload concurrency) and then verify, end to
//! end, that every pack actually landed and that the data round-trips
//! bit-for-bit — i.e. the concurrent uploads and the finalize barrier are
//! correct.

use std::{
    fs,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use bytes::Bytes;
use bytesize::ByteSize;
use pretty_assertions::assert_eq;
use rstest::rstest;
use tempfile::{TempDir, tempdir};

use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, Credentials, ErrorKind, FileType, Id, KeyOptions,
    LocalDestination, LsOptions, OpenStatus, ReadBackend, Repository, RepositoryBackends,
    RepositoryOptions, RestoreOptions, RusticError, RusticResult, WriteBackend,
    repofile::{MasterKey, PackId, SnapshotFile},
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

use super::RepoOpen;

/// A backend that delegates to an in-memory backend but fails the
/// `write_bytes` of the Nth (1-based) pack file, to exercise the upload error
/// path. Everything else is transparent.
#[derive(Debug)]
struct FailNthPackBackend {
    inner: InMemoryBackend,
    packs_written: AtomicUsize,
    fail_on_pack: usize,
}

impl FailNthPackBackend {
    fn new(fail_on_pack: usize) -> Self {
        Self {
            inner: InMemoryBackend::new(),
            packs_written: AtomicUsize::new(0),
            fail_on_pack,
        }
    }
}

impl ReadBackend for FailNthPackBackend {
    fn location(&self) -> String {
        self.inner.location()
    }
    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        self.inner.list_with_size(tpe)
    }
    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.inner.read_full(tpe, id)
    }
    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        self.inner.read_partial(tpe, id, cacheable, offset, length)
    }
    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.inner.warmup_path(tpe, id)
    }
}

impl WriteBackend for FailNthPackBackend {
    fn create(&self) -> RusticResult<()> {
        self.inner.create()
    }
    fn write_bytes(&self, tpe: FileType, id: &Id, cacheable: bool, buf: Bytes) -> RusticResult<()> {
        if tpe == FileType::Pack {
            let n = self.packs_written.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.fail_on_pack {
                return Err(RusticError::new(
                    ErrorKind::Backend,
                    "injected upload failure for testing",
                ));
            }
        }
        self.inner.write_bytes(tpe, id, cacheable, buf)
    }
    fn remove(&self, tpe: FileType, id: &Id, cacheable: bool) -> RusticResult<()> {
        self.inner.remove(tpe, id, cacheable)
    }
}

/// A tiny deterministic PRNG so we can generate incompressible, distinct file
/// content without pulling in the `rand` crate. (Also keeps the test
/// reproducible.)
fn fill_pseudo_random(buf: &mut [u8], seed: u64) {
    // splitmix64
    let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    for chunk in buf.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let bytes = z.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
}

/// Init an in-memory repo with a deliberately tiny data packsize, so a modest
/// amount of data produces many packs (well above the upload concurrency).
///
/// `upload_concurrency` is wired through [`RepositoryOptions`] exactly as it
/// would be from `rustic.toml`, so this also exercises the config plumbing.
fn small_packsize_repo(upload_concurrency: usize) -> Result<RepoOpen> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default().upload_concurrency(upload_concurrency);
    let repo: Repository<OpenStatus> = Repository::new(&options, &be)?.init(
        &Credentials::Masterkey(MasterKey::new()),
        &KeyOptions::default(),
        &ConfigOptions::default()
            // ~256 KiB packs, no growth: forces lots of small packs.
            .set_datapack_size(Some(ByteSize::kib(256)))
            .set_datapack_size_limit(Some(ByteSize::kib(256)))
            .set_datapack_growfactor(Some(0)),
    )?;
    Ok(repo)
}

/// Write `num_files` files of `file_size` bytes of distinct pseudo-random data.
fn make_source(num_files: usize, file_size: usize) -> Result<TempDir> {
    let dir = tempdir()?;
    let mut buf = vec![0u8; file_size];
    for i in 0..num_files {
        fill_pseudo_random(&mut buf, i as u64 + 1);
        fs::write(dir.path().join(format!("file_{i:04}.bin")), &buf)?;
    }
    Ok(dir)
}

/// `backup` -> `check(read_data)` -> `restore` -> compare, with enough data to
/// span many packs. If the parallel writer dropped, reordered, or failed to
/// finalize any upload, `check --read-data` or the byte comparison catches it.
///
/// Parametrized over the upload concurrency: `1` proves the sequential edge
/// case still round-trips, and higher values exercise the concurrent path.
#[rstest]
#[case(1)]
#[case(4)]
#[case(16)]
fn test_parallel_upload_roundtrip(#[case] upload_concurrency: usize) -> Result<()> {
    // 64 files x 256 KiB = 16 MiB of incompressible data. With ~256 KiB packs
    // that is on the order of 60+ data packs vs. the upload concurrency.
    let num_files = 64;
    let file_size = 256 * 1024;
    let source = make_source(num_files, file_size)?;
    let source_paths =
        rustic_core::PathList::from_iter(Some(source.path().to_path_buf()));

    let repo = small_packsize_repo(upload_concurrency)?.to_indexed_ids()?;
    let backup_opts = BackupOptions::default().as_path(PathBuf::from_str("src")?);
    let snapshot = repo.backup(&backup_opts, &source_paths, SnapshotFile::default())?;

    // Sanity: we really did produce many more packs than the upload
    // concurrency (8), so the concurrent path was genuinely exercised. If this
    // ever drops to a handful, the test has degraded and is no longer a
    // meaningful concurrency test.
    assert!(
        snapshot.summary.as_ref().unwrap().data_added_files_packed > 0,
        "expected data to be added"
    );
    let pack_count = repo.list::<PackId>()?.count();
    assert!(
        pack_count >= 16,
        "expected many packs to exercise concurrency, got {pack_count}"
    );

    // Full integrity check including re-reading + hashing every pack.
    let repo = repo.to_indexed()?;
    repo.check(CheckOptions::default().read_data(true))?
        .is_ok()?;

    // Restore and compare every file byte-for-byte against the source.
    let node = repo.node_from_snapshot_path("latest", |_| true)?;
    let ls = repo.ls(&node, &LsOptions::default())?;
    let restore_dir = tempdir()?;
    let dest = LocalDestination::new(
        restore_dir.path().to_str().unwrap(),
        true,
        !node.is_dir(),
    )?;
    let restore_opts = RestoreOptions::default();
    let plan = repo.prepare_restore(&restore_opts, ls.clone(), &dest, false)?;
    repo.restore(plan, &restore_opts, ls, &dest)?;

    let restored_root = restore_dir.path().join("src");
    for i in 0..num_files {
        let name = format!("file_{i:04}.bin");
        let original = fs::read(source.path().join(&name))?;
        let restored = fs::read(restored_root.join(&name))?;
        assert_eq!(original, restored, "content mismatch for {name}");
    }

    Ok(())
}

/// If a pack upload fails mid-stream while several uploads are in flight, the
/// backup must abort with an error and must NOT commit a snapshot — otherwise a
/// snapshot could reference a pack that never landed. This is the corruption
/// scenario the parallel writer must not introduce.
#[rstest]
fn test_upload_failure_aborts_without_snapshot() -> Result<()> {
    let source = make_source(64, 256 * 1024)?;
    let source_paths = rustic_core::PathList::from_iter(Some(source.path().to_path_buf()));

    // Fail the 3rd pack upload, with 8 uploads potentially in flight.
    let be = FailNthPackBackend::new(3);
    let be = RepositoryBackends::new(Arc::new(be), None);
    let repo: Repository<OpenStatus> = Repository::new(
        &RepositoryOptions::default().upload_concurrency(8usize),
        &be,
    )?
    .init(
        &Credentials::Masterkey(MasterKey::new()),
        &KeyOptions::default(),
        &ConfigOptions::default()
            .set_datapack_size(Some(ByteSize::kib(256)))
            .set_datapack_size_limit(Some(ByteSize::kib(256)))
            .set_datapack_growfactor(Some(0)),
    )?;
    let repo = repo.to_indexed_ids()?;

    let backup_opts = BackupOptions::default().as_path(PathBuf::from_str("src")?);
    let result = repo.backup(&backup_opts, &source_paths, SnapshotFile::default());

    // The backup must surface the injected error...
    assert!(
        result.is_err(),
        "backup should fail when a pack upload fails"
    );

    // ...and must not have committed any snapshot referencing the missing data.
    let snapshots = repo.get_all_snapshots()?;
    assert!(
        snapshots.is_empty(),
        "no snapshot must be committed on upload failure, found {}",
        snapshots.len()
    );

    Ok(())
}
