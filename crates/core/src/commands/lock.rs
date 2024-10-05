//! `lock` subcommand

use chrono::{DateTime, Local};
use log::error;
use rayon::ThreadPoolBuilder;

use crate::{
    error::{CommandErrorKind, RepositoryErrorKind, RusticResult},
    progress::{Progress, ProgressBars},
    repofile::{configfile::ConfigId, IndexId, KeyId, PackId, RepoId, SnapshotId},
    repository::Repository,
    WriteBackend,
};

pub(super) mod constants {
    /// The maximum number of reader threads to use for locking.
    pub(super) const MAX_LOCKER_THREADS_NUM: usize = 20;
}

pub fn lock_repo<P: ProgressBars, S>(
    repo: &Repository<P, S>,
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    lock_all_files::<P, S, ConfigId>(repo, until)?;
    lock_all_files::<P, S, KeyId>(repo, until)?;
    lock_all_files::<P, S, SnapshotId>(repo, until)?;
    lock_all_files::<P, S, IndexId>(repo, until)?;
    lock_all_files::<P, S, PackId>(repo, until)?;
    Ok(())
}

pub fn lock_all_files<P: ProgressBars, S, ID: RepoId + std::fmt::Debug>(
    repo: &Repository<P, S>,
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    if !repo.be.can_lock() {
        return Err(CommandErrorKind::NoLockingConfigured.into());
    }

    let p = &repo
        .pb
        .progress_spinner(format!("listing {:?} files..", ID::TYPE));
    let ids: Vec<ID> = repo.list()?.collect();
    p.finish();
    lock_files(repo, &ids, until)
}

fn lock_files<P: ProgressBars, S, ID: RepoId + std::fmt::Debug>(
    repo: &Repository<P, S>,
    ids: &[ID],
    until: Option<DateTime<Local>>,
) -> RusticResult<()> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(constants::MAX_LOCKER_THREADS_NUM)
        .build()
        .map_err(RepositoryErrorKind::FromThreadPoolbilderError)?;
    let p = &repo
        .pb
        .progress_counter(format!("locking {:?} files..", ID::TYPE));
    p.set_length(ids.len().try_into().unwrap());
    let backend = &repo.be;
    pool.in_place_scope(|scope| {
        for id in ids {
            scope.spawn(move |_| {
                if let Err(e) = backend.lock(ID::TYPE, id, until) {
                    // FIXME: Use error handling
                    error!("lock failed for {:?} {id:?}. {e}", ID::TYPE);
                };
                p.inc(1);
            });
        }
    });
    p.finish();
    Ok(())
}
