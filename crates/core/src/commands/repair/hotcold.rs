use std::collections::{BTreeMap, BTreeSet};

use log::{debug, info, warn};

use crate::{
    backend::decrypt::DecryptReadBackend,
    repofile::{BlobType, IndexFile, PackId},
    repository::Open,
    ErrorKind, FileType, Id, Progress, ProgressBars, ReadBackend, Repository, RusticError,
    RusticResult, WriteBackend, ALL_FILE_TYPES,
};

/// Repairs a hot/cold repository by copying missing files (except pack files) over from one to the other part.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository
/// * `dry_run` - Do a dry run
pub(crate) fn repair_hotcold<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    dry_run: bool,
) -> RusticResult<()> {
    for file_type in ALL_FILE_TYPES {
        if file_type != FileType::Pack {
            correct_missing_files(repo, file_type, |_| true, dry_run)?;
        }
    }
    Ok(())
}

/// Repairs a hot/cold repository by copying missing tree pack files over from one to the other part.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository
/// * `dry_run` - Do a dry run
pub(crate) fn repair_hotcold_packs<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
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

/// Copy relevant+misssing files in a hot/cold repository from one to the other part.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository
/// * `file_type` - The filetype to copy
/// * `is_relevalt` - A closure to determine whether the id is relevat
/// * `dry_run` - Do a dry run
pub(crate) fn correct_missing_files<P: ProgressBars, S>(
    repo: &Repository<P, S>,
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
            let p = repo
                .pb
                .progress_bytes(format!("copying missing cold {file_type:?} files..."));
            p.set_length(missing_cold_size);
            copy(missing_cold, file_type, repo_hot, &repo.be_cold)?;
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
            // TODO: warm-up
            // copy missing files from cold to hot repo
            let p = repo
                .pb
                .progress_bytes(format!("copying missing hot {file_type:?} files..."));
            p.set_length(missing_hot_size);
            copy(missing_hot, file_type, &repo.be_cold, repo_hot)?;
            p.finish();
        }
    }

    Ok(())
}

/// Copy a list of files from one repo to another.
///
/// # Arguments
///
/// * `files` - The list of file ids to copy
/// * `file_type` - The filetype to copy
/// * `from` - The backend to read from
/// * `to` - The backend to write to
fn copy(
    files: Vec<Id>,
    file_type: FileType,
    from: &impl ReadBackend,
    to: &impl WriteBackend,
) -> RusticResult<()> {
    for id in files {
        let file = from.read_full(file_type, &id)?;
        to.write_bytes(file_type, &id, false, file)?;
    }
    Ok(())
}

/// Get all tree packs from from within the repository.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository
///
/// # Returns
///
/// The set of pack ids.
pub(crate) fn get_tree_packs<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
) -> RusticResult<BTreeSet<PackId>> {
    let p = repo.pb.progress_counter("reading index...");
    let mut tree_packs = BTreeSet::new();
    for index in repo.dbe().stream_all::<IndexFile>(&p)? {
        let index = index?.1;
        for (p, _) in index.all_packs() {
            let blob_type = p.blob_type();
            if blob_type == BlobType::Tree {
                _ = tree_packs.insert(p.id);
            }
        }
    }
    Ok(tree_packs)
}

/// Find missing files in the hot or cold part of the repository.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository
/// * `file_type` - The filetype to use
/// * `is_relevalt` - A closure to determine whether the id is relevat
///
/// # Returns
///
/// A tuple containing ids missing in hot part, the total size, ids missing in cold part and the corresponding total size.
pub(crate) fn get_missing_files<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    file_type: FileType,
    is_relevant: impl Fn(&Id) -> bool,
) -> RusticResult<(Vec<Id>, u64, Vec<Id>, u64)> {
    let Some(repo_hot) = &repo.be_hot else {
        return Err(RusticError::new(
            ErrorKind::Repository,
            "Repository is no hot/cold repository.",
        ));
    };

    let p = repo
        .pb
        .progress_spinner(format!("listing hot {file_type:?} files..."));
    let hot_files: BTreeMap<_, _> = repo_hot.list_with_size(file_type)?.into_iter().collect();
    p.finish();

    let p = repo
        .pb
        .progress_spinner(format!("listing cold {file_type:?} files..."));
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
