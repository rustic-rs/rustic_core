use std::fmt;

use crate::repofile::SnapshotFile;

use runtime_format::{FormatKey, FormatKeyError};

/// A formatted snapshot.
///
/// To be formatted with [`runtime_format`].
///
/// The following keys are available:
/// - `id`: the snapshot id
/// - `long_id`: the snapshot id as a string
/// - `time`: the snapshot time
/// - `username`: the snapshot username
/// - `hostname`: the snapshot hostname
/// - `label`: the snapshot label
/// - `tags`: the snapshot tags
/// - `backup_start`: the snapshot backup start time
/// - `backup_end`: the snapshot backup end time
#[derive(Debug)]
pub(crate) struct FormattedSnapshot<'a> {
    /// The snapshot file.
    pub(crate) snap: &'a SnapshotFile,
    /// The time format to use.
    pub(crate) time_format: &'a str,
}

impl<'a> FormatKey for FormattedSnapshot<'a> {
    fn fmt(&self, key: &str, f: &mut fmt::Formatter<'_>) -> Result<(), FormatKeyError> {
        match key {
            "id" => write!(f, "{}", self.snap.id),
            "long_id" => write!(f, "{:?}", self.snap.id),
            "time" => write!(f, "{}", self.snap.time.format(self.time_format)),
            "username" => write!(f, "{}", self.snap.username),
            "hostname" => write!(f, "{}", self.snap.hostname),
            "label" => write!(f, "{}", self.snap.label),
            "tags" => write!(f, "{}", self.snap.tags),
            "backup_start" => {
                if let Some(summary) = &self.snap.summary {
                    write!(f, "{}", summary.backup_start.format(self.time_format))
                } else {
                    write!(f, "no_backup_start")
                }
            }
            "backup_end" => {
                if let Some(summary) = &self.snap.summary {
                    write!(f, "{}", summary.backup_end.format(self.time_format))
                } else {
                    write!(f, "no_backup_end")
                }
            }

            _ => return Err(FormatKeyError::UnknownKey),
        }
        .map_err(FormatKeyError::Fmt)
    }
}
