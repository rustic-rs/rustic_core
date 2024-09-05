use std::ops::Deref;

use serde::{de::DeserializeOwned, Serialize};
use typed_id::TypedId;

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
pub trait RepoId: Deref<Target = Id> + Sized + Send + Sync + 'static {
    /// The [`FileType`] associated with Id type
    const TYPE: FileType;
}

// Auto-implement RepoId for RepoFiles
impl<F: RepoFile> RepoId for TypedId<Id, F> {
    const TYPE: FileType = F::TYPE;
}

// Part of public API

use crate::Id;

pub use {
    crate::{
        backend::{
            node::{Metadata, Node, NodeType},
            FileType, ALL_FILE_TYPES,
        },
        blob::{tree::Tree, BlobType, ALL_BLOB_TYPES},
    },
    configfile::ConfigFile,
    indexfile::{IndexBlob, IndexFile, IndexPack},
    keyfile::KeyFile,
    packfile::{HeaderEntry, PackHeader, PackHeaderLength, PackHeaderRef},
    snapshotfile::{DeleteOption, PathList, SnapshotFile, SnapshotSummary, StringList},
};
