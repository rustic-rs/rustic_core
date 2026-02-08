use std::{
    cmp::Ordering,
    fmt::{self, Display},
    str::FromStr,
};

use derive_setters::Setters;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::{
    ForgetSnapshot, StringList,
    repofile::{
        SnapshotFile,
        snapshotfile::{SnapshotFileErrorKind, SnapshotFileResult},
    },
};

/// [`SnapshotGroupCriterion`] determines how to group snapshots.
///
/// `Default` grouping is by hostname, label and paths.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Copy, Setters, Deserialize, Serialize)]
#[setters(into)]
#[non_exhaustive]
pub struct SnapshotGroupCriterion {
    /// Whether to group by hostnames
    pub hostname: bool,

    /// Whether to group by labels
    pub label: bool,

    /// Whether to group by paths
    pub paths: bool,

    /// Whether to group by tags
    pub tags: bool,
}

impl SnapshotGroupCriterion {
    /// Create a new empty `SnapshotGroupCriterion`
    #[must_use]
    pub fn new() -> Self {
        Self {
            hostname: false,
            label: false,
            paths: false,
            tags: false,
        }
    }

    /// Create a `SnapshotGroupCriterion` from a `SnapshotGroup`
    #[must_use]
    pub fn from_group(group: &SnapshotGroup) -> Self {
        Self {
            hostname: group.hostname.is_some(),
            label: group.label.is_some(),
            paths: group.paths.is_some(),
            tags: group.tags.is_some(),
        }
    }
}

impl Default for SnapshotGroupCriterion {
    fn default() -> Self {
        Self {
            hostname: true,
            label: true,
            paths: true,
            tags: false,
        }
    }
}

impl FromStr for SnapshotGroupCriterion {
    type Err = SnapshotFileErrorKind;
    fn from_str(s: &str) -> SnapshotFileResult<Self> {
        let mut crit = Self::new();
        for val in s.split(',') {
            match val {
                "host" => crit.hostname = true,
                "label" => crit.label = true,
                "paths" => crit.paths = true,
                "tags" => crit.tags = true,
                "" => {}
                v => return Err(SnapshotFileErrorKind::ValueNotAllowed(v.into())),
            }
        }
        Ok(crit)
    }
}

impl Display for SnapshotGroupCriterion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut display = Vec::new();
        if self.hostname {
            display.push("host");
        }
        if self.label {
            display.push("label");
        }
        if self.paths {
            display.push("paths");
        }
        if self.tags {
            display.push("tags");
        }
        write!(f, "{}", display.join(","))?;
        Ok(())
    }
}

#[skip_serializing_none]
#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
/// [`SnapshotGroup`] specifies the group after a grouping using [`SnapshotGroupCriterion`].
pub struct SnapshotGroup {
    /// Group hostname, if grouped by hostname
    pub hostname: Option<String>,

    /// Group label, if grouped by label
    pub label: Option<String>,

    /// Group paths, if grouped by paths
    pub paths: Option<StringList>,

    /// Group tags, if grouped by tags
    pub tags: Option<StringList>,
}

impl PartialOrd for SnapshotGroup {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SnapshotGroup {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hostname
            .cmp(&other.hostname)
            .then(self.label.cmp(&other.label))
            .then(self.paths.cmp(&other.paths))
            .then(self.tags.cmp(&other.tags))
    }
}

impl Display for SnapshotGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = Vec::new();

        if let Some(host) = &self.hostname {
            out.push(format!("host [{host}]"));
        }
        if let Some(label) = &self.label {
            out.push(format!("label [{label}]"));
        }
        if let Some(paths) = &self.paths {
            out.push(format!("paths [{paths}]"));
        }
        if let Some(tags) = &self.tags {
            out.push(format!("tags [{tags}]"));
        }

        write!(f, "({})", out.join(", "))?;
        Ok(())
    }
}

impl SnapshotGroup {
    /// Extracts the suitable [`SnapshotGroup`] from a [`SnapshotFile`] using a given [`SnapshotGroupCriterion`].
    ///
    /// # Arguments
    ///
    /// * `sn` - The [`SnapshotFile`] to extract the [`SnapshotGroup`] from
    /// * `crit` - The [`SnapshotGroupCriterion`] to use
    #[must_use]
    pub fn from_snapshot(sn: &SnapshotFile, crit: SnapshotGroupCriterion) -> Self {
        Self {
            hostname: crit.hostname.then(|| sn.hostname.clone()),
            label: crit.label.then(|| sn.label.clone()),
            paths: crit.paths.then(|| sn.paths.clone()),
            tags: crit.tags.then(|| sn.tags.clone()),
        }
    }

    /// Check if the [`SnapshotFile`] is in the [`SnapshotGroup`].
    ///
    /// # Arguments
    ///
    /// * `group` - The [`SnapshotGroup`] to check
    #[must_use]
    pub fn matches(&self, snapshot: &SnapshotFile) -> bool {
        self.hostname
            .as_ref()
            .is_none_or(|val| val == &snapshot.hostname)
            && self.label.as_ref().is_none_or(|val| val == &snapshot.label)
            && self.paths.as_ref().is_none_or(|val| val == &snapshot.paths)
            && self.tags.as_ref().is_none_or(|val| val == &snapshot.tags)
    }

    /// Returns whether this is an empty group, i.e. no grouping information is contained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

pub trait Grouping {
    type GroupKey: PartialEq + Ord + fmt::Debug;
    type Criterion: Copy;
    fn get_group(&self, c: Self::Criterion) -> Self::GroupKey;
}

impl Grouping for SnapshotFile {
    type GroupKey = SnapshotGroup;
    type Criterion = SnapshotGroupCriterion;
    fn get_group(&self, c: Self::Criterion) -> Self::GroupKey {
        SnapshotGroup::from_snapshot(self, c)
    }
}

impl Grouping for ForgetSnapshot {
    type GroupKey = SnapshotGroup;
    type Criterion = SnapshotGroupCriterion;
    fn get_group(&self, c: Self::Criterion) -> Self::GroupKey {
        SnapshotGroup::from_snapshot(&self.snapshot, c)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
/// A group is a `Vec` of items with identical `group_key`
pub struct Group<T: Grouping> {
    /// The key for this group
    pub group_key: T::GroupKey,
    /// The items of the group
    pub items: Vec<T>,
}

impl<T: Grouping> Group<T>
where
    T::GroupKey: Default,
{
    /// A group where `group_key` is the default for this group key
    #[must_use]
    pub fn default_group(items: Vec<T>) -> Self {
        Self {
            group_key: T::GroupKey::default(),
            items,
        }
    }
}

#[derive(Debug)]
/// A grouped list of items
pub struct Grouped<T: Grouping> {
    /// The criterion used for groupung
    pub criterion: T::Criterion,
    /// The groups
    pub groups: Vec<Group<T>>,
}

impl<T: Grouping> Grouped<T> {
    /// Create a new empty group of snapshots
    #[must_use]
    pub fn new(criterion: T::Criterion) -> Self {
        Self {
            criterion,
            groups: Vec::new(),
        }
    }

    /// Crate a group of items by grouping them with `criterion`
    #[must_use]
    pub fn from_items(mut items: Vec<T>, criterion: T::Criterion) -> Self {
        items.sort_unstable_by_key(|item| item.get_group(criterion));
        let mut groups = Vec::new();
        for (group, snaps) in &items.into_iter().chunk_by(|item| item.get_group(criterion)) {
            groups.push(Group {
                group_key: group,
                items: snaps.collect(),
            });
        }
        Self { criterion, groups }
    }

    /// Update the group using `update` on the `Vec` of items
    ///
    /// # Errors
    ///
    /// * If `update` returns an error
    pub fn try_update_with<E>(
        self,
        update: impl FnOnce(Vec<T>) -> Result<Vec<T>, E>,
    ) -> Result<Self, E> {
        let crit = self.criterion;
        let items = update(self.into())?;
        Ok(Self::from_items(items, crit))
    }
}

impl<T: Grouping> From<Grouped<T>> for Vec<T> {
    fn from(value: Grouped<T>) -> Self {
        value
            .groups
            .into_iter()
            .flat_map(|group| group.items)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        "host,label,paths",
        true,
        true,
        true,
        false,
        "host,label,paths",
        "(host [myhost], label [mylabel], paths [/path])"
    )]
    #[case("host", true, false, false, false, "host", "(host [myhost])")]
    #[case(
        "label,host",
        true,
        true,
        false,
        false,
        "host,label",
        "(host [myhost], label [mylabel])"
    )]
    #[case("tags", false, false, false, true, "tags", "(tags [tag1,tag2])")]
    #[case(
        "paths,label",
        false,
        true,
        true,
        false,
        "label,paths",
        "(label [mylabel], paths [/path])"
    )]
    fn fromstr_display(
        #[case] input: String,
        #[case] is_host: bool,
        #[case] is_label: bool,
        #[case] is_path: bool,
        #[case] is_tags: bool,
        #[case] display: String,
        #[case] group_display: String,
    ) {
        let crit: SnapshotGroupCriterion = input.parse().unwrap();
        assert_eq!(crit.hostname, is_host);
        assert_eq!(crit.label, is_label);
        assert_eq!(crit.paths, is_path);
        assert_eq!(crit.tags, is_tags);

        assert_eq!(crit.to_string(), display);

        let sn = SnapshotFile {
            hostname: "myhost".to_string(),
            label: "mylabel".to_string(),
            paths: "/path".parse().unwrap(),
            tags: "tag1,tag2".parse().unwrap(),
            ..Default::default()
        };

        let group = SnapshotGroup::from_snapshot(&sn, crit);
        assert_eq!(group.to_string(), group_display);
    }
}
