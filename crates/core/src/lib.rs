/*!
A library for deduplicated and encrypted backups, using repositories as specified in the [`restic repository design`](https://github.com/restic/restic/blob/master/doc/design.rst).

# Overview

This section gives a brief overview of the primary types in this crate:

The main type is the [`Repository`] type which describes a way to access a repository.
It can be in different states and allows - depending on the state - various high-level
actions to be performed on the repository like listing snapshots, backing up or restoring.

Besides this, various `*Option` types exist which allow to specify options for accessing a
[`Repository`] or for the methods used within a [`Repository`]. Those types usually offer
setter methods as well as implement [`serde::Serialize`] and [`serde::Deserialize`].

Other main types are typically result types obtained by [`Repository`] methods which sometimes
are also needed as input for other [`Repository`] method, like computing a [`PrunePlan`] and
performing it.

There are also lower level data types which represent the stored repository format or
help accessing/writing it. Those are collected in the [`repofile`] module. These types typically
implement [`serde::Serialize`] and [`serde::Deserialize`].

# Example - initialize a repository, backup to it and get snapshots

```rust
    use rustic_backend::BackendOptions;
    use rustic_core::{BackupOptions, ConfigOptions, KeyOptions, PathList,
        Repository, RepositoryOptions, SnapshotOptions
    };

    // Initialize the repository in a temporary dir
    let repo_dir = tempfile::tempdir().unwrap();

    let repo_opts = RepositoryOptions::default()
        .password("test");

    // Initialize Backends
    let backends = BackendOptions::default()
        .repository(repo_dir.path().to_str().unwrap())
        .to_backends()
        .unwrap();

    let key_opts = KeyOptions::default();

    let config_opts = ConfigOptions::default();

    let _repo = Repository::new(&repo_opts, &backends.clone()).unwrap().init(&key_opts, &config_opts).unwrap();

    // We could have used _repo directly, but open the repository again to show how to open it...
    let repo = Repository::new(&repo_opts, &backends).unwrap().open().unwrap();

    // Get all snapshots from the repository
    let snaps = repo.get_all_snapshots().unwrap();
    // Should be zero, as the repository has just been initialized
    assert_eq!(snaps.len(), 0);

    // Turn repository state to indexed (for backup):
    let repo = repo.to_indexed_ids().unwrap();

    // Pre-define the snapshot-to-backup
    let snap = SnapshotOptions::default()
        .add_tags("tag1,tag2").unwrap()
        .to_snapshot().unwrap();

    // Specify backup options and source
    let backup_opts = BackupOptions::default();
    let source = PathList::from_string("src").unwrap().sanitize().unwrap();

    // run the backup and return the snapshot pointing to the backup'ed data.
    let snap = repo.backup(&backup_opts, &source, snap).unwrap();
    // assert_eq!(&snap.paths, ["src"]);

    // Get all snapshots from the repository
    let snaps = repo.get_all_snapshots().unwrap();
    // Should now be 1, we just created a snapshot
    assert_eq!(snaps.len(), 1);

    assert_eq!(snaps[0], snap);
```

# Crate features

This crate exposes a few features for controlling dependency usage.

- **cli** - Enables support for CLI features by enabling `clap` and `merge`
  features. *This feature is disabled by default*.

- **clap** - Enables a dependency on the `clap` crate and enables parsing from
    the commandline. *This feature is disabled by default*.

- **merge** - Enables support for merging multiple values into one, which
  enables the `merge` dependency. This is needed for parsing commandline
  arguments and merging them into one (e.g. `config`). *This feature is disabled
  by default*.

- **webdav** - Enables a dependency on the `dav-server` and `futures` crate.
  This enables us to run a `WebDAV` server asynchronously on the commandline.
  *This feature is disabled by default*.
*/

// Workspace lints don't seem to work for this?
#![allow(clippy::redundant_pub_crate)]

pub(crate) mod archiver;
pub(crate) mod backend;
pub(crate) mod blob;
pub(crate) mod cdc;
pub(crate) mod chunker;
pub(crate) mod commands;
pub(crate) mod crypto;
pub(crate) mod error;
pub(crate) mod id;
pub(crate) mod index;
pub(crate) mod progress;
/// Structs which are saved in JSON or binary format in the repository
pub mod repofile;
pub(crate) mod repository;
/// Virtual File System support - allows to act on the repository like on a file system
pub mod vfs;

// rustic_core Public API
pub use crate::{
    backend::{
        decrypt::{compression_level_range, max_compression_level},
        ignore::{LocalSource, LocalSourceFilterOptions, LocalSourceSaveOptions},
        local_destination::LocalDestination,
        node::last_modified_node,
        FileType, ReadBackend, ReadSource, ReadSourceEntry, ReadSourceOpen, RepositoryBackends,
        WriteBackend, ALL_FILE_TYPES,
    },
    blob::{
        tree::{FindMatches, FindNode, TreeId, TreeStreamerOptions as LsOptions},
        BlobId, DataId,
    },
    commands::{
        backup::{BackupOptions, ParentOptions},
        check::CheckOptions,
        config::ConfigOptions,
        copy::CopySnapshot,
        forget::{ForgetGroup, ForgetGroups, ForgetSnapshot, KeepOptions},
        key::KeyOptions,
        prune::{LimitOption, PruneOptions, PrunePlan, PruneStats},
        repair::{index::RepairIndexOptions, snapshots::RepairSnapshotsOptions},
        repoinfo::{BlobInfo, IndexInfos, PackInfo, RepoFileInfo, RepoFileInfos},
        restore::{FileDirStats, RestoreOptions, RestorePlan, RestoreStats},
    },
    error::{RusticError, RusticResult},
    id::{HexId, Id},
    progress::{NoProgress, NoProgressBars, Progress, ProgressBars},
    repofile::snapshotfile::{
        PathList, SnapshotGroup, SnapshotGroupCriterion, SnapshotOptions, StringList,
    },
    repository::{
        CommandInput, FullIndex, IndexedFull, IndexedIds, IndexedStatus, IndexedTree, OpenStatus,
        Repository, RepositoryOptions,
    },
};
