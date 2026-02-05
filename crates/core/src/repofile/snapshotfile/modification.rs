use std::path::PathBuf;

use cached::proc_macro::cached;
use derive_setters::Setters;
use jiff::{Span, Zoned};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    ErrorKind, RusticError, RusticResult, StringList,
    repofile::{DeleteOption, RusticTime, SnapshotFile},
};

/// Modification(s) to apply to a snapshot
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[serde_as]
#[derive(Debug, Clone, Default, Setters, Serialize, Deserialize)]
#[setters(into)]
#[non_exhaustive]
pub struct SnapshotModification {
    /// Set label
    #[cfg_attr(feature = "clap", clap(long, value_name = "LABEL"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_label: Option<String>,

    /// Set the backup time (e.g. "2021-01-21 14:15:23")
    #[cfg_attr(feature = "clap", clap(long, value_parser = crate::repofile::RusticTime::parse_system))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde_as(as = "Option<RusticTime>")]
    pub set_time: Option<Zoned>,

    /// Set the host name
    #[cfg_attr(feature = "clap", clap(long, value_name = "NAME"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_hostname: Option<String>,

    /// Tags to add (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "TAG[,TAG,..]", conflicts_with = "remove_tags")
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub add_tags: Vec<StringList>,

    /// Tag list to set (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "TAG[,TAG,..]", conflicts_with = "remove_tags")
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub set_tags: Vec<StringList>,

    /// Tags to remove (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long, value_name = "TAG[,TAG,..]"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub remove_tags: Vec<StringList>,

    /// Set description
    #[cfg_attr(feature = "clap", clap(long, value_name = "DESCRIPTION"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_description: Option<String>,

    /// Read description to set from the given file
    #[cfg_attr(
        feature = "clap",
        clap(long, value_name = "FILE", conflicts_with = "set_description", value_hint = clap::ValueHint::FilePath)
     )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub set_description_from: Option<PathBuf>,

    /// Remove description
    #[cfg_attr(feature = "clap", clap(long, conflicts_with_all = &["set_description", "set_description_from"]))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub remove_description: bool,

    /// Mark snapshot to be deleted after given duration (e.g. 10d)
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub set_delete_after: Option<Span>,

    /// Mark snapshot as uneraseable
    #[cfg_attr(feature = "clap", clap(long, conflicts_with = "set_delete_after"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub set_delete_never: bool,

    /// Remove any delete mark
    #[cfg_attr(feature = "clap", clap(long, conflicts_with_all = &["set_delete_never", "set_delete_after"]))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub remove_delete: bool,
}

// cache description if read from file
#[cached(size = 1)]
fn get_description_from_file(path: PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("{err:?}"))
}

impl SnapshotModification {
    /// Apply this modification(s) to the given snapshot
    ///
    /// # Returns
    /// `true` if the snapshot was changed.
    ///
    /// # Errors
    /// if reading a description from a file failed
    pub fn apply_to(&self, sn: &mut SnapshotFile) -> RusticResult<bool> {
        let delete = match (
            self.remove_delete,
            self.set_delete_never,
            self.set_delete_after,
        ) {
            (true, _, _) => Some(DeleteOption::NotSet),
            (_, true, _) => Some(DeleteOption::Never),
            (_, _, Some(d)) => Some(DeleteOption::After(Zoned::now() + d)),
            (false, false, None) => None,
        };

        let description = match (self.remove_description, &self.set_description_from) {
            (true, _) => Some(None),
            (false, Some(path)) => Some(Some(get_description_from_file(path.clone()).map_err(
                |err| {
                    RusticError::with_source(
                        ErrorKind::Other,
                        "Failed to read description from file {path}.",
                        err,
                    )
                    .attach_context("path", path.to_string_lossy())
                },
            )?)),
            (false, None) => self
                .set_description
                .as_ref()
                .map(|description| Some(description.clone())),
        };

        let mut changed = false;

        if !self.set_tags.is_empty() {
            changed |= sn.set_tags(self.set_tags.clone());
        }
        changed |= sn.add_tags(self.add_tags.clone());
        changed |= sn.remove_tags(&self.remove_tags);
        changed |= set_check(&mut sn.delete, &delete);
        changed |= set_check(&mut sn.label, &self.set_label);
        changed |= set_check(&mut sn.description, &description);
        changed |= set_check(&mut sn.time, &self.set_time);
        changed |= set_check(&mut sn.hostname, &self.set_hostname);
        Ok(changed)
    }
}

#[allow(clippy::ref_option)]
fn set_check<T: PartialEq + Clone>(a: &mut T, b: &Option<T>) -> bool {
    if let Some(b) = b
        && *a != *b
    {
        *a = b.clone();
        return true;
    }
    false
}
