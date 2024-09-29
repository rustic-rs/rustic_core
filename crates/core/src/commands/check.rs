//! `check` subcommand
use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
};

use bytes::Bytes;
use bytesize::ByteSize;
use derive_setters::Setters;
use log::{debug, error, warn};
use rand::{prelude::SliceRandom, thread_rng, Rng};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use zstd::stream::decode_all;

use crate::{
    backend::{cache::Cache, decrypt::DecryptReadBackend, node::NodeType, FileType, ReadBackend},
    blob::{tree::TreeStreamerOnce, BlobId, BlobType},
    crypto::hasher::hash,
    error::{CommandErrorKind, RusticErrorKind, RusticResult},
    id::Id,
    index::{
        binarysorted::{IndexCollector, IndexType},
        GlobalIndex, ReadGlobalIndex,
    },
    progress::{Progress, ProgressBars},
    repofile::{
        packfile::PackId, IndexFile, IndexPack, PackHeader, PackHeaderLength, PackHeaderRef,
    },
    repository::{Open, Repository},
    TreeId,
};

#[derive(Clone, Copy, Debug, Default)]
/// Options to specify which subset of packs will be read
pub enum ReadSubsetOption {
    #[default]
    /// Read all pack files
    All,
    /// Read a random subset of pack files with (approximately) the given percentage of total size
    Percentage(f64),
    /// Read a random subset of pack files with (approximately) the given size
    Size(u64),
    /// Read a subset of packfiles based on Ids: Using (1,n) .. (n,n) in separate runs will cover all pack files
    IdSubSet((u32, u32)),
}

impl ReadSubsetOption {
    fn apply(self, packs: impl IntoIterator<Item = IndexPack>) -> Vec<IndexPack> {
        self.apply_with_rng(packs, &mut thread_rng())
    }

    fn apply_with_rng(
        self,
        packs: impl IntoIterator<Item = IndexPack>,
        rng: &mut impl Rng,
    ) -> Vec<IndexPack> {
        fn id_matches_n_m(id: &Id, n: u32, m: u32) -> bool {
            let short_id: u32 = id.transmute();
            short_id % m == n % m
        }

        let mut total_size: u64 = 0;
        let mut packs: Vec<_> = packs
            .into_iter()
            .inspect(|p| total_size += u64::from(p.pack_size()))
            .collect();

        // Apply read-subset option
        if let Some(mut size) = match self {
            Self::All => None,
            // we need some casts to compute percentage...
            #[allow(clippy::cast_possible_truncation)]
            #[allow(clippy::cast_precision_loss)]
            #[allow(clippy::cast_sign_loss)]
            Self::Percentage(p) => Some((total_size as f64 * p / 100.0) as u64),
            Self::Size(s) => Some(s),
            Self::IdSubSet((n, m)) => {
                packs.retain(|p| id_matches_n_m(&p.id, n, m));
                None
            }
        } {
            // random subset of given size is required
            packs.shuffle(rng);
            packs.retain(|p| {
                let p_size = u64::from(p.pack_size());
                if size > p_size {
                    size = size.saturating_sub(p_size);
                    true
                } else {
                    false
                }
            });
        }
        packs
    }
}

impl FromStr for ReadSubsetOption {
    type Err = CommandErrorKind;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let result = if s == "all" {
            Self::All
        } else if let Some(p) = s.strip_suffix('%') {
            // try to read percentage
            Self::Percentage(p.parse()?)
        } else if let Some((n, m)) = s.split_once('/') {
            // try to read n/m
            Self::IdSubSet((n.parse()?, m.parse()?))
        } else {
            Self::Size(
                ByteSize::from_str(s)
                    .map_err(CommandErrorKind::FromByteSizeParser)?
                    .as_u64(),
            )
        };
        Ok(result)
    }
}

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[derive(Clone, Copy, Debug, Default, Setters)]
#[setters(into)]
/// Options for the `check` command
pub struct CheckOptions {
    /// Don't verify the data saved in the cache
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "no_cache"))]
    pub trust_cache: bool,

    /// Also read and check pack files
    #[cfg_attr(feature = "clap", clap(long))]
    pub read_data: bool,

    /// Read and check pack files
    #[cfg_attr(feature = "clap", clap(long, default_value = "all"))]
    pub read_data_subset: ReadSubsetOption,
}

impl CheckOptions {
    /// Runs the `check` command
    ///
    /// # Type Parameters
    ///
    /// * `P` - The progress bar type.
    /// * `S` - The state the repository is in.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository to check
    ///
    /// # Errors
    ///
    /// If the repository is corrupted
    pub(crate) fn run<P: ProgressBars, S: Open>(
        self,
        repo: &Repository<P, S>,
        trees: Vec<TreeId>,
    ) -> RusticResult<()> {
        let be = repo.dbe();
        let cache = repo.cache();
        let hot_be = &repo.be_hot;
        let raw_be = repo.dbe();
        let pb = &repo.pb;
        if !self.trust_cache {
            if let Some(cache) = &cache {
                for file_type in [FileType::Snapshot, FileType::Index] {
                    // list files in order to clean up the cache
                    //
                    // This lists files here and later when reading index / checking snapshots
                    // TODO: Only list the files once...
                    _ = be
                        .list_with_size(file_type)
                        .map_err(RusticErrorKind::Backend)?;

                    let p = pb.progress_bytes(format!("checking {file_type:?} in cache..."));
                    // TODO: Make concurrency (20) customizable
                    check_cache_files(20, cache, raw_be, file_type, &p)?;
                }
            }
        }

        if let Some(hot_be) = hot_be {
            for file_type in [FileType::Snapshot, FileType::Index] {
                check_hot_files(raw_be, hot_be, file_type, pb)?;
            }
        }

        let index_collector = check_packs(be, hot_be, self.read_data, pb)?;

        if let Some(cache) = &cache {
            let p = pb.progress_spinner("cleaning up packs from cache...");
            let ids: Vec<_> = index_collector
                .tree_packs()
                .iter()
                .map(|(id, size)| (**id, *size))
                .collect();
            if let Err(err) = cache.remove_not_in_list(FileType::Pack, &ids) {
                warn!("Error in cache backend removing pack files: {err}");
            }
            p.finish();

            if !self.trust_cache {
                let p = pb.progress_bytes("checking packs in cache...");
                // TODO: Make concurrency (5) customizable
                check_cache_files(5, cache, raw_be, FileType::Pack, &p)?;
            }
        }

        let index_be = GlobalIndex::new_from_index(index_collector.into_index());

        let packs = check_trees(be, &index_be, trees, pb)?;

        if self.read_data {
            let packs = index_be
                .into_index()
                .into_iter()
                .filter(|p| packs.contains(&p.id));

            let packs = self.read_data_subset.apply(packs);

            repo.warm_up_wait(packs.iter().map(|pack| pack.id))?;

            let total_pack_size = packs.iter().map(|pack| u64::from(pack.pack_size())).sum();
            let p = pb.progress_bytes("reading pack data...");
            p.set_length(total_pack_size);

            packs.into_par_iter().for_each(|pack| {
                let id = pack.id;
                let data = be.read_full(FileType::Pack, &id).unwrap();
                match check_pack(be, pack, data, &p) {
                    Ok(()) => {}
                    Err(err) => error!("Error reading pack {id} : {err}",),
                }
            });
            p.finish();
        }
        Ok(())
    }
}

/// Checks if all files in the backend are also in the hot backend
///
/// # Arguments
///
/// * `be` - The backend to check
/// * `be_hot` - The hot backend to check
/// * `file_type` - The type of the files to check
/// * `pb` - The progress bar to use
///
/// # Errors
///
/// If a file is missing or has a different size
fn check_hot_files(
    be: &impl ReadBackend,
    be_hot: &impl ReadBackend,
    file_type: FileType,
    pb: &impl ProgressBars,
) -> RusticResult<()> {
    let p = pb.progress_spinner(format!("checking {file_type:?} in hot repo..."));
    let mut files = be
        .list_with_size(file_type)
        .map_err(RusticErrorKind::Backend)?
        .into_iter()
        .collect::<HashMap<_, _>>();

    let files_hot = be_hot
        .list_with_size(file_type)
        .map_err(RusticErrorKind::Backend)?;

    for (id, size_hot) in files_hot {
        match files.remove(&id) {
            None => error!("hot file Type: {file_type:?}, Id: {id} does not exist in repo"),
            Some(size) if size != size_hot => {
                // TODO: This should be an actual error not a log entry
                error!("Type: {file_type:?}, Id: {id}: hot size: {size_hot}, actual size: {size}");
            }
            _ => {} //everything ok
        }
    }

    for (id, _) in files {
        error!("hot file Type: {file_type:?}, Id: {id} is missing!",);
    }
    p.finish();

    Ok(())
}

/// Checks if all files in the cache are also in the backend
///
/// # Arguments
///
/// * `concurrency` - The number of threads to use
/// * `cache` - The cache to check
/// * `be` - The backend to check
/// * `file_type` - The type of the files to check
/// * `p` - The progress bar to use
///
/// # Errors
///
/// If a file is missing or has a different size
fn check_cache_files(
    _concurrency: usize,
    cache: &Cache,
    be: &impl ReadBackend,
    file_type: FileType,
    p: &impl Progress,
) -> RusticResult<()> {
    let files = cache.list_with_size(file_type)?;

    if files.is_empty() {
        return Ok(());
    }

    let total_size = files.values().map(|size| u64::from(*size)).sum();
    p.set_length(total_size);

    files
        .into_par_iter()
        .for_each_with((cache, be, p.clone()), |(cache, be, p), (id, size)| {
            // Read file from cache and from backend and compare
            match (
                cache.read_full(file_type, &id),
                be.read_full(file_type, &id),
            ) {
                (Err(err), _) => {
                    error!("Error reading cached file Type: {file_type:?}, Id: {id} : {err}");
                }
                (_, Err(err)) => {
                    error!("Error reading file Type: {file_type:?}, Id: {id} : {err}");
                }
                (Ok(Some(data_cached)), Ok(data)) if data_cached != data => {
                    error!(
                        "Cached file Type: {file_type:?}, Id: {id} is not identical to backend!"
                    );
                }
                (Ok(_), Ok(_)) => {} // everything ok
            }

            p.inc(u64::from(size));
        });

    p.finish();
    Ok(())
}

/// Check if packs correspond to index and are present in the backend
///
/// # Arguments
///
/// * `be` - The backend to check
/// * `hot_be` - The hot backend to check
/// * `read_data` - Whether to read the data of the packs
/// * `pb` - The progress bar to use
///
/// # Errors
///
/// If a pack is missing or has a different size
///
/// # Returns
///
/// The index collector
fn check_packs(
    be: &impl DecryptReadBackend,
    hot_be: &Option<impl ReadBackend>,
    read_data: bool,
    pb: &impl ProgressBars,
) -> RusticResult<IndexCollector> {
    let mut packs = HashMap::new();
    let mut tree_packs = HashMap::new();
    let mut index_collector = IndexCollector::new(if read_data {
        IndexType::Full
    } else {
        IndexType::DataIds
    });

    let p = pb.progress_counter("reading index...");
    for index in be.stream_all::<IndexFile>(&p)? {
        let index = index?.1;
        index_collector.extend(index.packs.clone());
        for (p, to_delete) in index.all_packs() {
            let check_time = to_delete; // Check if time is set for packs marked to delete
            let blob_type = p.blob_type();
            let pack_size = p.pack_size();
            _ = packs.insert(p.id, pack_size);
            if hot_be.is_some() && blob_type == BlobType::Tree {
                _ = tree_packs.insert(p.id, pack_size);
            }

            // Check if time is set _
            if check_time && p.time.is_none() {
                error!("pack {}: No time is set! Run prune to correct this!", p.id);
            }

            // check offsests in index
            let mut expected_offset: u32 = 0;
            let mut blobs = p.blobs;
            blobs.sort_unstable();
            for blob in blobs {
                if blob.tpe != blob_type {
                    error!(
                        "pack {}: blob {} blob type does not match: type: {:?}, expected: {:?}",
                        p.id, blob.id, blob.tpe, blob_type
                    );
                }

                if blob.offset != expected_offset {
                    error!(
                        "pack {}: blob {} offset in index: {}, expected: {}",
                        p.id, blob.id, blob.offset, expected_offset
                    );
                }
                expected_offset += blob.length;
            }
        }
    }

    p.finish();

    if let Some(hot_be) = hot_be {
        let p = pb.progress_spinner("listing packs in hot repo...");
        check_packs_list(hot_be, tree_packs)?;
        p.finish();
    }

    let p = pb.progress_spinner("listing packs...");
    check_packs_list(be, packs)?;
    p.finish();

    Ok(index_collector)
}

// TODO: Add documentation
/// Checks if all packs in the backend are also in the index
///
/// # Arguments
///
/// * `be` - The backend to check
/// * `packs` - The packs to check
///
/// # Errors
///
/// If a pack is missing or has a different size
fn check_packs_list(be: &impl ReadBackend, mut packs: HashMap<PackId, u32>) -> RusticResult<()> {
    for (id, size) in be
        .list_with_size(FileType::Pack)
        .map_err(RusticErrorKind::Backend)?
    {
        match packs.remove(&PackId::from(id)) {
            None => warn!("pack {id} not referenced in index. Can be a parallel backup job. To repair: 'rustic repair index'."),
            Some(index_size) if index_size != size => {
                error!("pack {id}: size computed by index: {index_size}, actual size: {size}. To repair: 'rustic repair index'.");
            }
            _ => {} //everything ok
        }
    }

    for (id, _) in packs {
        error!("pack {id} is referenced by the index but not present! To repair: 'rustic repair index'.",);
    }
    Ok(())
}

/// Check if all snapshots and contained trees can be loaded and contents exist in the index
///
/// # Arguments
///
/// * `index` - The index to check
/// * `pb` - The progress bar to use
///
/// # Errors
///
/// If a snapshot or tree is missing or has a different size
fn check_trees(
    be: &impl DecryptReadBackend,
    index: &impl ReadGlobalIndex,
    snap_trees: Vec<TreeId>,
    pb: &impl ProgressBars,
) -> RusticResult<BTreeSet<PackId>> {
    let mut packs = BTreeSet::new();
    let p = pb.progress_counter("checking trees...");
    let mut tree_streamer = TreeStreamerOnce::new(be, index, snap_trees, p)?;
    while let Some(item) = tree_streamer.next().transpose()? {
        let (path, tree) = item;
        for node in tree.nodes {
            match node.node_type {
                NodeType::File => node.content.as_ref().map_or_else(
                    || {
                        error!("file {:?} doesn't have a content", path.join(node.name()));
                    },
                    |content| {
                        for (i, id) in content.iter().enumerate() {
                            if id.is_null() {
                                error!("file {:?} blob {} has null ID", path.join(node.name()), i);
                            }

                            match index.get_data(id) {
                                None => {
                                    error!(
                                        "file {:?} blob {} is missing in index",
                                        path.join(node.name()),
                                        id
                                    );
                                }
                                Some(entry) => {
                                    _ = packs.insert(entry.pack);
                                }
                            }
                        }
                    },
                ),

                NodeType::Dir => {
                    match node.subtree {
                        None => {
                            error!("dir {:?} subtree does not exist", path.join(node.name()));
                        }
                        Some(tree) if tree.is_null() => {
                            error!("dir {:?} subtree has null ID", path.join(node.name()));
                        }
                        Some(id) => match index.get_tree(&id) {
                            None => {
                                error!(
                                    "dir {:?} subtree blob {} is missing in index",
                                    path.join(node.name()),
                                    id
                                );
                            }
                            Some(entry) => {
                                _ = packs.insert(entry.pack);
                            }
                        }, // subtree is ok
                    }
                }

                _ => {} // nothing to check
            }
        }
    }

    Ok(packs)
}

/// Check if a pack is valid
///
/// # Arguments
///
/// * `be` - The backend to use
/// * `index_pack` - The pack to check
/// * `data` - The data of the pack
/// * `p` - The progress bar to use
///
/// # Errors
///
/// If the pack is invalid
///
/// # Panics
///
/// If zstd decompression fails.
fn check_pack(
    be: &impl DecryptReadBackend,
    index_pack: IndexPack,
    mut data: Bytes,
    p: &impl Progress,
) -> RusticResult<()> {
    let id = index_pack.id;
    let size = index_pack.pack_size();
    if data.len() != size as usize {
        error!(
            "pack {id}: data size does not match expected size. Read: {} bytes, expected: {size} bytes",
            data.len()
        );
        return Ok(());
    }

    let comp_id = PackId::from(hash(&data));
    if id != comp_id {
        error!("pack {id}: Hash mismatch. Computed hash: {comp_id}");
        return Ok(());
    }

    // check header length
    let header_len = PackHeaderRef::from_index_pack(&index_pack).size();
    let pack_header_len = PackHeaderLength::from_binary(&data.split_off(data.len() - 4))?.to_u32();
    if pack_header_len != header_len {
        error!("pack {id}: Header length in pack file doesn't match index. In pack: {pack_header_len}, calculated: {header_len}");
        return Ok(());
    }

    // check header
    let header = be.decrypt(&data.split_off(data.len() - header_len as usize))?;

    let pack_blobs = PackHeader::from_binary(&header)?.into_blobs();
    let mut blobs = index_pack.blobs;
    blobs.sort_unstable_by_key(|b| b.offset);
    if pack_blobs != blobs {
        error!("pack {id}: Header from pack file does not match the index");
        debug!("pack file header: {pack_blobs:?}");
        debug!("index: {:?}", blobs);
        return Ok(());
    }
    p.inc(u64::from(header_len) + 4);

    // check blobs
    for blob in blobs {
        let blob_id = blob.id;
        let mut blob_data = be.decrypt(&data.split_to(blob.length as usize))?;

        // TODO: this is identical to backend/decrypt.rs; unify these two parts!
        if let Some(length) = blob.uncompressed_length {
            blob_data = decode_all(&*blob_data).unwrap();
            if blob_data.len() != length.get() as usize {
                error!("pack {id}, blob {blob_id}: Actual uncompressed length does not fit saved uncompressed length");
                return Ok(());
            }
        }

        let comp_id = BlobId::from(hash(&blob_data));
        if blob.id != comp_id {
            error!("pack {id}, blob {blob_id}: Hash mismatch. Computed hash: {comp_id}");
            return Ok(());
        }
        p.inc(blob.length.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_ron_snapshot;
    use rand::{rngs::StdRng, Rng, SeedableRng};
    use rstest::{fixture, rstest};

    const PACK_SIZE: u32 = 100_000_000;

    #[fixture]
    fn rng() -> StdRng {
        StdRng::seed_from_u64(5)
    }
    fn test_packs(rng: &mut impl Rng) -> Vec<IndexPack> {
        (0..500)
            .map(|_| IndexPack {
                id: PackId::from(Id::random_from_rng(rng)),
                blobs: Vec::new(),
                time: None,
                size: Some(rng.gen_range(0..PACK_SIZE)),
            })
            .collect()
    }

    #[rstest]
    #[case("all")]
    #[case("5/12")]
    #[case("5%")]
    #[case("250MiB")]
    fn test_read_subset(mut rng: StdRng, #[case] s: &str) {
        let size =
            |packs: &[IndexPack]| -> u64 { packs.iter().map(|p| u64::from(p.pack_size())).sum() };

        let test_packs = test_packs(&mut rng);
        let total_size = size(&test_packs);

        let subset: ReadSubsetOption = s.parse().unwrap();
        let packs = subset.apply_with_rng(test_packs, &mut rng);
        let test_size = size(&packs);

        match subset {
            ReadSubsetOption::All => assert_eq!(test_size, total_size),
            #[allow(clippy::cast_possible_truncation)]
            #[allow(clippy::cast_precision_loss)]
            #[allow(clippy::cast_sign_loss)]
            ReadSubsetOption::Percentage(s) => assert!(test_size <= (total_size as f64 * s) as u64),
            ReadSubsetOption::Size(size) => {
                assert!(test_size <= size && size <= test_size + u64::from(PACK_SIZE));
            }
            ReadSubsetOption::IdSubSet(_) => {}
        };

        let ids: Vec<_> = packs.iter().map(|pack| pack.id).collect();
        assert_ron_snapshot!(s, ids);
    }

    fn test_read_subset_n_m() {
        let test_packs = test_packs(&mut thread_rng());
        let mut all_packs: BTreeSet<_> = test_packs.iter().map(|pack| pack.id).collect();

        let mut run_with = |s: &str| {
            let subset: ReadSubsetOption = s.parse().unwrap();
            let packs = subset.apply(test_packs.clone());
            for pack in packs {
                assert!(all_packs.remove(&pack.id));
            }
        };

        run_with("1/5");
        run_with("2/5");
        run_with("3/5");
        run_with("4/5");
        run_with("5/5");

        assert!(all_packs.is_empty());
    }
}
