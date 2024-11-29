//! `forget` subcommand

use chrono::{DateTime, Datelike, Duration, Local, Timelike};
use derive_setters::Setters;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};

use crate::{
    error::{ErrorKind, RusticError, RusticResult},
    progress::ProgressBars,
    repofile::{
        snapshotfile::{SnapshotGroup, SnapshotGroupCriterion, SnapshotId},
        SnapshotFile, StringList,
    },
    repository::{Open, Repository},
};

type CheckFunction = fn(&SnapshotFile, &SnapshotFile) -> bool;

#[derive(Debug, Serialize)]
/// A newtype for `[Vec<ForgetGroup>]`
pub struct ForgetGroups(pub Vec<ForgetGroup>);

#[derive(Debug, Serialize)]
/// All snapshots of a group with group and forget information
pub struct ForgetGroup {
    /// The group
    pub group: SnapshotGroup,
    /// The list of snapshots within this group
    pub snapshots: Vec<ForgetSnapshot>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
/// This struct enhances `[SnapshotFile]` with the attributes `keep` and `reasons` which indicates if the snapshot should be kept and why.
pub struct ForgetSnapshot {
    /// The snapshot
    pub snapshot: SnapshotFile,
    /// Whether it should be kept
    pub keep: bool,
    /// reason(s) for keeping / not keeping the snapshot
    pub reasons: Vec<String>,
}

impl ForgetGroups {
    /// Turn `ForgetGroups` into the list of all snapshot IDs to remove.
    #[must_use]
    pub fn into_forget_ids(self) -> Vec<SnapshotId> {
        self.0
            .into_iter()
            .flat_map(|fg| {
                fg.snapshots
                    .into_iter()
                    .filter_map(|fsn| (!fsn.keep).then_some(fsn.snapshot.id))
            })
            .collect()
    }
}

/// Get the list of snapshots to forget.
///
/// # Type Parameters
///
/// * `P` - The progress bar type.
/// * `S` - The state the repository is in.
///
/// # Arguments
///
/// * `repo` - The repository to use
/// * `keep` - The keep options to use
/// * `group_by` - The criterion to group snapshots by
/// * `filter` - The filter to apply to the snapshots
///
/// # Errors
///
/// * If keep options are not valid
///
/// # Returns
///
/// The list of snapshot groups with the corresponding snapshots and forget information
pub(crate) fn get_forget_snapshots<P: ProgressBars, S: Open>(
    repo: &Repository<P, S>,
    keep: &KeepOptions,
    group_by: SnapshotGroupCriterion,
    filter: impl FnMut(&SnapshotFile) -> bool,
) -> RusticResult<ForgetGroups> {
    let now = Local::now();

    let groups = repo
        .get_snapshot_group(&[], group_by, filter)?
        .into_iter()
        .map(|(group, snapshots)| -> RusticResult<_> {
            Ok(ForgetGroup {
                group,
                snapshots: keep.apply(snapshots, now)?,
            })
        })
        .collect::<RusticResult<_>>()?;

    Ok(ForgetGroups(groups))
}

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[skip_serializing_none]
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
/// Options which snapshots should be kept. Used by the `forget` command.
pub struct KeepOptions {
    /// Keep snapshots with this taglist (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long, value_name = "TAG[,TAG,..]"))]
    #[cfg_attr(feature = "merge", merge(strategy=conflate::vec::overwrite_empty))]
    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keep_tags: Vec<StringList>,

    /// Keep snapshots ids that start with ID (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long = "keep-id", value_name = "ID"))]
    #[cfg_attr(feature = "merge", merge(strategy=conflate::vec::overwrite_empty))]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keep_ids: Vec<String>,

    /// Keep the last N snapshots (N == -1: keep all snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'l', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_last: Option<i32>,

    /// Keep the last N minutely snapshots (N == -1: keep all minutely snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'M', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_minutely: Option<i32>,

    /// Keep the last N hourly snapshots (N == -1: keep all hourly snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'H', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_hourly: Option<i32>,

    /// Keep the last N daily snapshots (N == -1: keep all daily snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'd', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_daily: Option<i32>,

    /// Keep the last N weekly snapshots (N == -1: keep all weekly snapshots)
    #[cfg_attr(
        feature = "clap",
        clap(long, short = 'w', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_weekly: Option<i32>,

    /// Keep the last N monthly snapshots (N == -1: keep all monthly snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'm', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_monthly: Option<i32>,

    /// Keep the last N quarter-yearly snapshots (N == -1: keep all quarter-yearly snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_quarter_yearly: Option<i32>,

    /// Keep the last N half-yearly snapshots (N == -1: keep all half-yearly snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_half_yearly: Option<i32>,

    /// Keep the last N yearly snapshots (N == -1: keep all yearly snapshots)
    #[cfg_attr(
        feature = "clap", 
        clap(long, short = 'y', value_name = "N",  allow_hyphen_values = true, value_parser = clap::value_parser!(i32).range(-1..))
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_yearly: Option<i32>,

    /// Keep snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within: Option<humantime::Duration>,

    /// Keep minutely snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_minutely: Option<humantime::Duration>,

    /// Keep hourly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_hourly: Option<humantime::Duration>,

    /// Keep daily snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_daily: Option<humantime::Duration>,

    /// Keep weekly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_weekly: Option<humantime::Duration>,

    /// Keep monthly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_monthly: Option<humantime::Duration>,

    /// Keep quarter-yearly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_quarter_yearly: Option<humantime::Duration>,

    /// Keep half-yearly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_half_yearly: Option<humantime::Duration>,

    /// Keep yearly snapshots newer than DURATION relative to latest snapshot
    #[cfg_attr(feature = "clap", clap(long, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub keep_within_yearly: Option<humantime::Duration>,

    /// Allow to keep no snapshot
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy=conflate::bool::overwrite_false))]
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_none: bool,
}

/// Always return false
///
/// # Arguments
///
/// * `_sn1` - The first snapshot
/// * `_sn2` - The second snapshot
const fn always_false(_sn1: &SnapshotFile, _sn2: &SnapshotFile) -> bool {
    false
}

/// Evaluate the year of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the year of the snapshots is equal
fn equal_year(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year()
}

/// Evaluate the half year of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the half year of the snapshots is equal
fn equal_half_year(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.month0() / 6 == t2.month0() / 6
}

/// Evaluate the quarter year of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the quarter year of the snapshots is equal
fn equal_quarter_year(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.month0() / 3 == t2.month0() / 3
}

/// Evaluate the month of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the month of the snapshots is equal
fn equal_month(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.month() == t2.month()
}

/// Evaluate the week of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the week of the snapshots is equal
fn equal_week(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.iso_week().week() == t2.iso_week().week()
}

/// Evaluate the day of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the day of the snapshots is equal
fn equal_day(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.ordinal() == t2.ordinal()
}

/// Evaluate the hours of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the hours of the snapshots are equal
fn equal_hour(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year() && t1.ordinal() == t2.ordinal() && t1.hour() == t2.hour()
}

/// Evaluate the minutes of the given snapshots
///
/// # Arguments
///
/// * `sn1` - The first snapshot
/// * `sn2` - The second snapshot
///
/// # Returns
///
/// Whether the minutes of the snapshots are equal
fn equal_minute(sn1: &SnapshotFile, sn2: &SnapshotFile) -> bool {
    let (t1, t2) = (sn1.time, sn2.time);
    t1.year() == t2.year()
        && t1.ordinal() == t2.ordinal()
        && t1.hour() == t2.hour()
        && t1.minute() == t2.minute()
}

impl KeepOptions {
    /// Check if `KeepOptions` are valid, i.e. if at least one keep-* option is given.
    fn is_valid(&self) -> bool {
        !self.keep_tags.is_empty()
            || !self.keep_ids.is_empty()
            || self.keep_last.is_some()
            || self.keep_minutely.is_some()
            || self.keep_hourly.is_some()
            || self.keep_daily.is_some()
            || self.keep_weekly.is_some()
            || self.keep_monthly.is_some()
            || self.keep_quarter_yearly.is_some()
            || self.keep_half_yearly.is_some()
            || self.keep_within.is_some()
            || self.keep_yearly.is_some()
            || self.keep_within_minutely.is_some()
            || self.keep_within_hourly.is_some()
            || self.keep_within_daily.is_some()
            || self.keep_within_weekly.is_some()
            || self.keep_within_monthly.is_some()
            || self.keep_within_quarter_yearly.is_some()
            || self.keep_within_half_yearly.is_some()
            || self.keep_within_yearly.is_some()
            || self.keep_none
    }

    /// Check if the given snapshot matches the keep options.
    ///
    /// # Arguments
    ///
    /// * `sn` - The snapshot to check
    /// * `last` - The last snapshot
    /// * `has_next` - Whether there is a next snapshot
    /// * `latest_time` - The time of the latest snapshot
    ///
    /// # Returns
    ///
    /// The list of reasons why the snapshot should be kept
    fn matches(
        &mut self,
        sn: &SnapshotFile,
        last: Option<&SnapshotFile>,
        has_next: bool,
        latest_time: DateTime<Local>,
    ) -> Vec<&str> {
        type MatchParameters<'a> = (
            CheckFunction,
            &'a mut Option<i32>,
            &'a str,
            Option<humantime::Duration>,
            &'a str,
        );

        let mut reason = Vec::new();

        let snapshot_id_hex = sn.id.to_hex();
        if self
            .keep_ids
            .iter()
            .any(|id| snapshot_id_hex.starts_with(id))
        {
            reason.push("id");
        }

        if !self.keep_tags.is_empty() && sn.tags.matches(&self.keep_tags) {
            reason.push("tags");
        }

        let keep_checks: [MatchParameters<'_>; 9] = [
            (
                always_false,
                &mut self.keep_last,
                "last",
                self.keep_within,
                "within",
            ),
            (
                equal_minute,
                &mut self.keep_minutely,
                "minutely",
                self.keep_within_minutely,
                "within minutely",
            ),
            (
                equal_hour,
                &mut self.keep_hourly,
                "hourly",
                self.keep_within_hourly,
                "within hourly",
            ),
            (
                equal_day,
                &mut self.keep_daily,
                "daily",
                self.keep_within_daily,
                "within daily",
            ),
            (
                equal_week,
                &mut self.keep_weekly,
                "weekly",
                self.keep_within_weekly,
                "within weekly",
            ),
            (
                equal_month,
                &mut self.keep_monthly,
                "monthly",
                self.keep_within_monthly,
                "within monthly",
            ),
            (
                equal_quarter_year,
                &mut self.keep_quarter_yearly,
                "quarter-yearly",
                self.keep_within_quarter_yearly,
                "within quarter-yearly",
            ),
            (
                equal_half_year,
                &mut self.keep_half_yearly,
                "half-yearly",
                self.keep_within_half_yearly,
                "within half-yearly",
            ),
            (
                equal_year,
                &mut self.keep_yearly,
                "yearly",
                self.keep_within_yearly,
                "within yearly",
            ),
        ];

        for (check_fun, counter, reason1, within, reason2) in keep_checks {
            if !has_next || last.is_none() || !check_fun(sn, last.unwrap()) {
                if let Some(counter) = counter {
                    if *counter != 0 {
                        reason.push(reason1);
                        if *counter > 0 {
                            *counter -= 1;
                        }
                    }
                }
                if let Some(within) = within {
                    if sn.time + Duration::from_std(*within).unwrap() > latest_time {
                        reason.push(reason2);
                    }
                }
            }
        }
        reason
    }

    /// Apply the `[KeepOptions]` to the given list of [`SnapshotFile`]s returning the corresponding
    /// list of [`ForgetSnapshot`]s
    ///
    /// # Arguments
    ///
    /// * `snapshots` - The list of snapshots to apply the options to
    /// * `now` - The current time
    ///
    /// # Errors
    ///
    /// * If keep options are not valid
    ///
    /// # Returns
    ///
    /// The list of snapshots with the attribute `keep` set to `true` if the snapshot should be kept and
    /// `reasons` set to the list of reasons why the snapshot should be kept
    pub fn apply(
        &self,
        mut snapshots: Vec<SnapshotFile>,
        now: DateTime<Local>,
    ) -> RusticResult<Vec<ForgetSnapshot>> {
        if !self.is_valid() {
            return Err(RusticError::new(
                ErrorKind::InvalidInput,
                "Invalid keep options specified, please make sure to specify at least one keep-* option.",
            ));
        }

        let mut group_keep = self.clone();
        let mut snaps = Vec::new();
        if snapshots.is_empty() {
            return Ok(snaps);
        }

        snapshots.sort_unstable_by(|sn1, sn2| sn1.cmp(sn2).reverse());
        let latest_time = snapshots[0].time;
        let mut last = None;

        let mut iter = snapshots.into_iter().peekable();

        while let Some(sn) = iter.next() {
            let (keep, reasons) = {
                if sn.must_keep(now) {
                    (true, vec!["snapshot"])
                } else if sn.must_delete(now) {
                    (false, vec!["snapshot"])
                } else {
                    let reasons =
                        group_keep.matches(&sn, last.as_ref(), iter.peek().is_some(), latest_time);
                    let keep = !reasons.is_empty();
                    (keep, reasons)
                }
            };
            last = Some(sn.clone());

            snaps.push(ForgetSnapshot {
                snapshot: sn,
                keep,
                reasons: reasons.iter().map(ToString::to_string).collect(),
            });
        }
        Ok(snaps)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::repofile::DeleteOption;

    use super::*;
    use anyhow::Result;
    use chrono::{Local, NaiveDateTime, TimeZone, Utc};
    use humantime::Duration;
    use insta::{assert_ron_snapshot, Settings};
    use rstest::{fixture, rstest};
    use serde_json;

    /// for more readable insta output
    #[derive(Serialize)]
    struct ForgetResult(Vec<(DateTime<Utc>, bool, Vec<String>)>);

    // helper for parsing times
    fn parse_time(time: &str) -> Result<DateTime<Local>> {
        let time = NaiveDateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S")?;
        Ok(Local::from_utc_datetime(&Local, &time))
    }

    #[fixture]
    fn test_snapshots() -> Vec<SnapshotFile> {
        let by_date = [
            "2014-09-01 10:20:30",
            "2014-09-02 10:20:30",
            "2014-09-05 10:20:30",
            "2014-09-06 10:20:30",
            "2014-09-08 10:20:30",
            "2014-09-09 10:20:30",
            "2014-09-10 10:20:30",
            "2014-09-11 10:20:30",
            "2014-09-20 10:20:30",
            "2014-09-22 10:20:30",
            "2014-08-08 10:20:30",
            "2014-08-10 10:20:30",
            "2014-08-12 10:20:30",
            "2014-08-13 10:20:30",
            "2014-08-15 10:20:30",
            "2014-08-18 10:20:30",
            "2014-08-20 10:20:30",
            "2014-08-21 10:20:30",
            "2014-08-22 10:20:30",
            "2014-11-18 10:20:30",
            "2014-11-20 10:20:30",
            "2014-11-21 10:20:30",
            "2014-11-22 10:20:30",
            "2015-09-01 10:20:30",
            "2015-09-02 10:20:30",
            "2015-09-05 10:20:30",
            "2015-09-06 10:20:30",
            "2015-09-08 10:20:30",
            "2015-09-09 10:20:30",
            "2015-09-10 10:20:30",
            "2015-09-11 10:20:30",
            "2015-09-20 10:20:30",
            "2015-09-22 10:20:30",
            "2015-08-08 10:20:30",
            "2015-08-10 10:20:30",
            "2015-08-12 10:20:30",
            "2015-08-13 10:20:30",
            "2015-08-15 10:20:30",
            "2015-08-18 10:20:30",
            "2015-08-20 10:20:30",
            "2015-08-21 10:20:30",
            "2015-08-22 10:20:30",
            "2015-10-01 10:20:30",
            "2015-10-02 10:20:30",
            "2015-10-05 10:20:30",
            "2015-10-06 10:20:30",
            "2015-10-08 10:20:30",
            "2015-10-09 10:20:30",
            "2015-10-10 10:20:30",
            "2015-10-11 10:20:30",
            "2015-10-20 10:20:30",
            "2015-10-22 10:20:30",
            "2015-10-22 10:20:30",
            "2015-11-08 10:20:30",
            "2015-11-10 10:20:30",
            "2015-11-12 10:20:30",
            "2015-11-13 10:20:30",
            "2015-11-15 10:20:30",
            "2015-11-18 10:20:30",
            "2015-11-20 10:20:30",
            "2015-11-21 10:20:30",
            "2015-11-22 10:20:30",
            "2016-01-01 01:02:03",
            "2016-01-01 01:03:03",
            "2016-01-01 07:08:03",
            "2016-01-03 07:02:03",
            "2016-01-04 10:23:03",
            "2016-01-04 11:23:03",
            "2016-01-04 12:24:03",
            "2016-01-04 12:28:03",
            "2016-01-04 12:30:03",
            "2016-01-04 16:23:03",
            "2016-01-07 10:02:03",
            "2016-01-08 20:02:03",
            "2016-01-09 21:02:03",
            "2016-01-12 21:02:03",
            "2016-01-12 21:08:03",
            "2016-01-18 12:02:03",
        ];

        let by_date_and_id = [
            (
                "2016-01-05 09:02:03",
                "23ef833f60639018019262ac63be5b87601ab58d23880bf6a474adea83dbbf8b",
            ),
            (
                "2016-01-06 08:02:03",
                "aca6165188e4ee770bb5c7a959a7c6612121960360a2f898203dc67dd75be8da",
            ),
            (
                "2016-01-04 12:23:03",
                "23ef833d367ddd53bb95cdad23207a1323b770494eae746496094f1db2416c5c",
            ),
        ];

        let by_date_and_tag = [
            ("2014-10-01 10:20:31", "foo"),
            ("2014-10-02 10:20:31", "foo"),
            ("2014-10-05 10:20:31", "foo"),
            ("2014-10-06 10:20:31", "foo"),
            ("2014-10-08 10:20:31", "foo"),
            ("2014-10-09 10:20:31", "foo"),
            ("2014-10-10 10:20:31", "foo"),
            ("2014-10-11 10:20:31", "foo"),
            ("2014-10-20 10:20:31", "foo"),
            ("2014-10-22 10:20:31", "foo"),
            ("2014-11-08 10:20:31", "foo"),
            ("2014-11-10 10:20:31", "foo"),
            ("2014-11-12 10:20:31", "foo"),
            ("2014-11-13 10:20:31", "foo"),
            ("2014-11-15 10:20:31", "foo,bar"),
            ("2015-10-22 10:20:31", "foo,bar"),
            ("2015-10-22 10:20:31", "foo,bar"),
        ];

        let delete_never = ["2014-09-01 10:25:37"];

        let delete_at = [
            ("2014-09-01 10:28:37", "2014-09-01 10:28:37"),
            ("2014-09-01 10:29:37", "2025-09-01 10:29:37"),
        ];

        let snaps: Vec<_> = by_date
            .into_iter()
            .map(|time| -> Result<_> {
                let opts = &crate::SnapshotOptions::default().time(parse_time(time)?);
                Ok(SnapshotFile::from_options(opts)?)
            })
            .chain(by_date_and_id.into_iter().map(|(time, id)| -> Result<_> {
                let opts = &crate::SnapshotOptions::default().time(parse_time(time)?);
                let mut snap = SnapshotFile::from_options(opts)?;
                snap.id = id.parse()?;
                Ok(snap)
            }))
            .chain(
                by_date_and_tag
                    .into_iter()
                    .map(|(time, tags)| -> Result<_> {
                        let opts = &crate::SnapshotOptions::default()
                            .time(parse_time(time)?)
                            .tags(vec![StringList::from_str(tags)?]);
                        Ok(SnapshotFile::from_options(opts)?)
                    }),
            )
            .chain(delete_never.into_iter().map(|time| -> Result<_> {
                let opts = &crate::SnapshotOptions::default().time(parse_time(time)?);
                let mut snap = SnapshotFile::from_options(opts)?;
                snap.delete = DeleteOption::Never;
                Ok(snap)
            }))
            .chain(delete_at.into_iter().map(|(time, delete)| -> Result<_> {
                let opts = &crate::SnapshotOptions::default().time(parse_time(time)?);
                let mut snap = SnapshotFile::from_options(opts)?;
                let delete = parse_time(delete)?;
                snap.delete = DeleteOption::After(delete);
                Ok(snap)
            }))
            .collect::<Result<_>>()
            .unwrap();

        snaps
    }

    #[fixture]
    fn insta_forget_snapshots_redaction() -> Settings {
        let mut settings = Settings::clone_current();
        settings.add_redaction(".**.snapshot", "[snapshot]");
        settings
    }

    #[test]
    fn apply_empty_snapshots() -> Result<()> {
        let now = Local::now();
        let options = KeepOptions::default().keep_last(10);
        let result = options.apply(vec![], now)?;
        assert!(result.is_empty());
        Ok(())
    }

    #[rstest]
    #[case(KeepOptions::default())]
    fn test_apply_fails(#[case] options: KeepOptions, test_snapshots: Vec<SnapshotFile>) {
        let now = Local::now();
        let result = options.apply(test_snapshots, now);
        assert!(result.is_err());
    }

    #[rstest]
    #[case(KeepOptions::default().keep_last(10))]
    #[case(KeepOptions::default().keep_last(15))]
    #[case(KeepOptions::default().keep_last(99))]
    #[case(KeepOptions::default().keep_last(200))]
    #[case(KeepOptions::default().keep_hourly(20))]
    #[case(KeepOptions::default().keep_daily(3))]
    #[case(KeepOptions::default().keep_daily(10))]
    #[case(KeepOptions::default().keep_daily(30))]
    #[case(KeepOptions::default().keep_last(5).keep_daily(5))]
    #[case(KeepOptions::default().keep_last(2).keep_daily(10))]
    #[case(KeepOptions::default().keep_weekly(2))]
    #[case(KeepOptions::default().keep_weekly(4))]
    #[case(KeepOptions::default().keep_daily(3).keep_weekly(4))]
    #[case(KeepOptions::default().keep_monthly(6))]
    #[case(KeepOptions::default().keep_daily(2).keep_weekly(2).keep_monthly(6))]
    #[case(KeepOptions::default().keep_yearly(10))]
    #[case(KeepOptions::default().keep_quarter_yearly(10))]
    #[case(KeepOptions::default().keep_half_yearly(10))]
    #[case(KeepOptions::default().keep_daily(7).keep_weekly(2).keep_monthly(3).keep_yearly(10))]
    #[case(KeepOptions::default().keep_tags(vec![StringList::from_str("foo")?]))]
    #[case(KeepOptions::default().keep_tags(vec![StringList::from_str("foo,bar")?]))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1d").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("2d").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("7d").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1m").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1M14d").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1y1d1M").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("13d23h").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("2M2h").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_hourly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_daily(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_weekly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_monthly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_quarter_yearly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_half_yearly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within_yearly(Duration::from_str("1y2M3d3h").unwrap()))]
    #[case(KeepOptions::default().keep_within(Duration::from_str("1h").unwrap()).keep_within_hourly(Duration::from_str("1d").unwrap()).keep_within_daily(Duration::from_str("1w").unwrap()).keep_within_weekly(Duration::from_str("1M").unwrap()).keep_within_monthly(Duration::from_str("1y").unwrap()).keep_within_yearly(Duration::from_str("9999y").unwrap()))]
    #[case(KeepOptions::default().keep_last(-1))]
    #[case(KeepOptions::default().keep_last(-1).keep_hourly(-1))]
    #[case(KeepOptions::default().keep_hourly(-1))]
    #[case(KeepOptions::default().keep_daily(3).keep_weekly(2).keep_monthly(-1).keep_yearly(-1))]
    #[case(KeepOptions::default().keep_ids(vec!["23ef".to_string()]))]
    #[case(KeepOptions::default().keep_none(true))]
    fn test_apply(
        #[case] options: KeepOptions,
        test_snapshots: Vec<SnapshotFile>,
        insta_forget_snapshots_redaction: Settings,
    ) -> Result<()> {
        let now = parse_time("2016-01-18 12:02:03")?;
        let result = options.apply(test_snapshots.clone(), now)?;

        // check that a changed current time doesn't change the forget result (note that DeleteOptions are set accordingly)
        let now = parse_time("2020-01-18 12:02:03")?;
        let result2 = options.apply(test_snapshots, now)?;
        assert_eq!(result, result2);

        // more readable output format
        let result = ForgetResult(
            result
                .into_iter()
                .map(|s| (s.snapshot.time.into(), s.keep, s.reasons))
                .collect(),
        );

        // good naming of snapshots: serialize into json and remove control chars
        let mut options = serde_json::to_string(&options)?;
        options.retain(|c| !"{}\":".contains(c));
        // shorten name, if too long
        if options.len() > 40 {
            options = options[..35].to_string();
            options.push_str("[cut]");
        }

        insta_forget_snapshots_redaction.bind(|| {
            assert_ron_snapshot!(options, result);
        });
        Ok(())
    }
}
