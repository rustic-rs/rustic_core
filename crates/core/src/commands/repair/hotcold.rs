use std::collections::{BTreeMap, BTreeSet};

use log::{debug, info, warn};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};

use crate::{
    ALL_FILE_TYPES, ErrorKind, FileType, Id, Progress, ReadBackend, Repository, RusticError,
    RusticResult, WriteBackend,
    backend::decrypt::DecryptReadBackend,
    repofile::{BlobType, IndexFile, PackId},
    repository::{Open, warm_up::warm_up_wait},
};

/// Repairs a hot/cold repository by copying missing files (except pack files) over from one to the other part.
pub(crate) fn repair_hotcold<S>(repo: &Repository<S>, dry_run: bool) -> RusticResult<()> {
    for file_type in ALL_FILE_TYPES {
        if file_type != FileType::Pack {
            correct_missing_files(repo, file_type, |_| true, dry_run)?;
        }
    }
    Ok(())
}

/// Repairs a hot/cold repository by copying missing tree packs over from one to the other part.
pub(crate) fn repair_hotcold_packs<S: Open>(
    repo: &Repository<S>,
    dry_run: bool,
) -> RusticResult<()> {
    let tree_packs = get_tree_packs(repo)?;
    correct_missing_files(
        repo,
        FileType::Pack,
        |id| tree_packs.contains(&PackId::from(*id)),
        dry_run,
    )
}

/// Copy relevant+missing files in a hot/cold repository from one to the other part.
pub(crate) fn correct_missing_files<S>(
    repo: &Repository<S>,
    file_type: FileType,
    is_relevant: impl Fn(&Id) -> bool,
    dry_run: bool,
) -> RusticResult<()> {
    let Some(repo_hot) = &repo.be_hot else {
        return Err(RusticError::new(
            ErrorKind::Repository,
            "Repository is no hot/cold repository.",
        ));
    };

    let (missing_hot, missing_hot_size, missing_cold, missing_cold_size) =
        get_missing_files(repo, file_type, is_relevant)?;

    // copy missing files from hot to cold repo
    if !missing_cold.is_empty() {
        if dry_run {
            info!(
                "would have copied {} hot {file_type:?} files to cold",
                missing_cold.len()
            );
            debug!("files: {missing_cold:?}");
        } else {
            let p = repo.progress_bytes(&format!("copying missing cold {file_type:?} files..."));
            p.set_length(missing_cold_size);
            copy(missing_cold, file_type, repo_hot, &repo.be_cold, &p)?;
            p.finish();
        }
    }

    if !missing_hot.is_empty() {
        if dry_run {
            info!(
                "would have copied {} cold {file_type:?} files to hot",
                missing_hot.len()
            );
            debug!("files: {missing_hot:?}");
        } else {
            warm_up_wait(repo, file_type, missing_hot.iter().copied())?;
            // copy missing files from cold to hot repo
            let p = repo.progress_bytes(&format!("copying missing hot {file_type:?} files..."));
            p.set_length(missing_hot_size);
            copy(missing_hot, file_type, &repo.be_cold, repo_hot, &p)?;
            p.finish();
        }
    }

    Ok(())
}

/// copy a list of files from one repository part to the other
fn copy(
    files: Vec<Id>,
    file_type: FileType,
    from: &impl ReadBackend,
    to: &impl WriteBackend,
    p: &Progress,
) -> RusticResult<()> {
    files.into_par_iter().try_for_each(|id| {
        let file = from.read_full(file_type, &id)?;
        let length = u64::try_from(file.len()).expect("file len should fit into u64");
        to.write_bytes(file_type, &id, false, file)?;
        p.inc(length);
        Ok(())
    })
}

/// Get all tree packs from within the repository
pub(crate) fn get_tree_packs<S: Open>(repo: &Repository<S>) -> RusticResult<BTreeSet<PackId>> {
    let p = repo.progress_counter("reading index...");
    let mut tree_packs = BTreeSet::new();
    for index in repo.dbe().stream_all::<IndexFile>(&p)? {
        let index = index?.1;
        for (pack, _) in index.all_packs() {
            let blob_type = pack.blob_type();
            if blob_type == BlobType::Tree {
                _ = tree_packs.insert(pack.id);
            }
        }
    }
    Ok(tree_packs)
}

/// Find missing files in the hot or cold part of a repository
///
/// # Returns
///
/// A tuble containing missing ids in the hot part; the corresponding total size; missing ids in the cold part; the corresponding total size.
pub(crate) fn get_missing_files<S>(
    repo: &Repository<S>,
    file_type: FileType,
    is_relevant: impl Fn(&Id) -> bool,
) -> RusticResult<(Vec<Id>, u64, Vec<Id>, u64)> {
    let Some(repo_hot) = &repo.be_hot else {
        return Err(RusticError::new(
            ErrorKind::Repository,
            "Repository is no hot/cold repository.",
        ));
    };

    let p = repo.progress_spinner(&format!("listing hot {file_type:?} files..."));
    let hot_files: BTreeMap<_, _> = repo_hot.list_with_size(file_type)?.into_iter().collect();
    p.finish();

    let p = repo.progress_spinner(&format!("listing cold {file_type:?} files..."));
    let cold_files: BTreeMap<_, _> = repo
        .be_cold
        .list_with_size(file_type)?
        .into_iter()
        .collect();
    p.finish();

    let common: BTreeSet<_> = hot_files
            .iter()
            .filter_map(|(id, size_hot)| match cold_files.get(id) {
                Some(size_cold) if size_cold == size_hot => Some(*id),
                Some(size_cold) => {
                     warn!("sizes mismatch: type {file_type:?}, id: {id}, size hot: {size_hot}, size cold: {size_cold}. Ignoring...");
                    None
                }
                None => None,
            })
            .collect();

    let retain = |files: BTreeMap<_, _>| {
        let mut retain_size: u64 = 0;
        let only: Vec<_> = files
            .into_iter()
            .filter(|(id, _)| !common.contains(id) && is_relevant(id))
            .map(|(id, size)| {
                retain_size += u64::from(size);
                id
            })
            .collect();
        (only, retain_size)
    };

    let (cold_only, cold_only_size) = retain(cold_files);
    let (hot_only, hot_only_size) = retain(hot_files);
    Ok((cold_only, cold_only_size, hot_only, hot_only_size))
}
