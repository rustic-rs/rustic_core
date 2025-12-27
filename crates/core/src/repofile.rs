use std::{borrow::Cow, ops::Deref, str::FromStr};

use jiff::{
    Timestamp, Zoned,
    civil::Time,
    fmt::temporal::{DateTimePrinter, Pieces},
    tz::TimeZone,
};
use serde::{Deserialize, Deserializer, Serialize, de, de::DeserializeOwned};
use serde_with::{DeserializeAs, SerializeAs};

pub(crate) mod configfile;
pub(crate) mod indexfile;
pub(crate) mod keyfile;
pub(crate) mod packfile;
pub(crate) mod snapshotfile;

/// Marker trait for repository files which are stored as JSON
pub trait RepoFile: Serialize + DeserializeOwned + Sized + Send + Sync + 'static {
    /// The [`FileType`] associated with the repository file
    const TYPE: FileType;
    /// Indicate whether the files are stored encrypted
    const ENCRYPTED: bool = true;
    /// The Id type associated with the repository file
    type Id: RepoId;
}

/// Marker trait for Ids which identify repository files
pub trait RepoId: Deref<Target = Id> + From<Id> + Sized + Copy + Send + Sync + 'static {
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

/// helper struct for serializing and deserializing
///
/// This is used in order to stay compatible with the restic repository format.
/// It can be directly used via `serde_as` or by using its methods for parsing and printing.
#[derive(Debug, Clone, Copy)]
pub struct RusticTime;

impl RusticTime {
    /// best-effort parsing of a string into a `Zoned`.
    ///
    /// # Errors
    pub fn parse(
        s: &str,
        default_time: Time,
        default_zone: TimeZone,
    ) -> Result<Zoned, jiff::Error> {
        if let Ok(zoned) = Zoned::from_str(s) {
            return Ok(zoned);
        }
        let pieces = Pieces::parse(&s)?;
        let time = pieces.time().unwrap_or(default_time);
        let dt = pieces.date().to_datetime(time);
        let zone = pieces.to_time_zone()?.unwrap_or_else(|| {
            pieces
                .to_numeric_offset()
                .map_or_else(|| default_zone, TimeZone::fixed)
        });
        dt.to_zoned(zone)
    }

    /// Best-effort parsing of a string into a `Zoned`.
    ///
    /// Uses 00:00 if no time is given and the system timezone if no zone is given.
    ///
    /// # Errors
    pub fn parse_system(s: &str) -> Result<Zoned, jiff::Error> {
        Self::parse(s, Time::MIN, TimeZone::system())
    }

    /// Best-effort parsing of a string into a `Zoned`.
    ///
    /// Uses 00:00 if no time is given and UTC if no zone is given.
    ///
    /// # Errors
    pub fn parse_utc(s: &str) -> Result<Zoned, jiff::Error> {
        Self::parse(s, Time::MIN, TimeZone::UTC)
    }

    /// Display a `Zoned` in a restic-compatible way, i.e. with offset, but without timezone
    #[must_use]
    pub fn to_string(source: &Zoned) -> String {
        DateTimePrinter::new().timestamp_with_offset_to_string(&source.timestamp(), source.offset())
    }
}

impl SerializeAs<Zoned> for RusticTime {
    fn serialize_as<S>(source: &Zoned, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&Self::to_string(source))
    }
}

impl<'de> DeserializeAs<'de, Zoned> for RusticTime {
    fn deserialize_as<D>(deserializer: D) -> Result<Zoned, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <Cow<'de, str>>::deserialize(deserializer)?;
        Self::parse_utc(&s).map_err(de::Error::custom)
    }
}

impl SerializeAs<Timestamp> for RusticTime {
    fn serialize_as<S>(source: &Timestamp, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let offset = TimeZone::system().to_offset(*source);
        serializer.collect_str(&source.display_with_offset(offset).to_string())
    }
}

impl<'de> DeserializeAs<'de, Timestamp> for RusticTime {
    fn deserialize_as<D>(deserializer: D) -> Result<Timestamp, D::Error>
    where
        D: Deserializer<'de>,
    {
        Timestamp::deserialize(deserializer)
    }
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
    configfile::{Chunker, ConfigFile},
    indexfile::{IndexBlob, IndexFile, IndexId, IndexPack},
    keyfile::{KeyFile, KeyId},
    packfile::{HeaderEntry, PackHeader, PackHeaderLength, PackHeaderRef, PackId},
    snapshotfile::{DeleteOption, PathList, SnapshotFile, SnapshotId, SnapshotSummary, StringList},
};
