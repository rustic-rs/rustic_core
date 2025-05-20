use std::ops::Deref;

use serde::{Serialize, de::DeserializeOwned};

pub(crate) mod configfile;
pub(crate) mod indexfile;
pub(crate) mod keyfile;
pub(crate) mod packfile;
pub(crate) mod snapshotfile;

/// Marker trait for repository files which are stored as encrypted JSON
pub trait RepoFile: Serialize + DeserializeOwned + Sized + Send + Sync + 'static {
    /// The [`FileType`] associated with the repository file
    const TYPE: FileType;
    /// The Id type associated with the repository file
    type Id: From<Id> + Send;
}

/// Marker trait for Ids which identify repository files
pub trait RepoId: Deref<Target = Id> + From<Id> + Sized + Send + Sync + 'static {
    /// The [`FileType`] associated with Id type
    const TYPE: FileType;
}

#[macro_export]
/// Generate newtypes for `Id`s identifying Repository files
macro_rules! impl_repoid {
    ($a:ident, $b: expr) => {
        $crate::define_new_id_struct!($a, concat!("repository file of type", stringify!($b)));
        impl $crate::repofile::RepoId for $a {
            const TYPE: FileType = $b;
        }
    };
}

#[macro_export]
/// Generate newtypes for `Id`s identifying Repository files implementing `RepoFile`
macro_rules! impl_repofile {
    ($a:ident, $b: expr, $c: ty) => {
        $crate::impl_repoid!($a, $b);
        impl RepoFile for $c {
            const TYPE: FileType = $b;
            type Id = $a;
        }
    };
}

// Part of public API
use crate::Id;

pub use {
    crate::{
        backend::{
            ALL_FILE_TYPES, FileType,
            node::{Metadata, Node, NodeType},
        },
        blob::{ALL_BLOB_TYPES, BlobType, tree::Tree},
    },
    configfile::ConfigFile,
    indexfile::{IndexBlob, IndexFile, IndexId, IndexPack},
    keyfile::{KeyFile, KeyId},
    packfile::{HeaderEntry, PackHeader, PackHeaderLength, PackHeaderRef, PackId},
    snapshotfile::{DeleteOption, PathList, SnapshotFile, SnapshotId, SnapshotSummary, StringList},
};
