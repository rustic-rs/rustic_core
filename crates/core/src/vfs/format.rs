use std::fmt;

use crate::repofile::SnapshotFile;

use runtime_format::{FormatKey, FormatKeyError};

pub(crate) struct FormattedSnapshot<'a> {
    pub(crate) snap: &'a SnapshotFile,
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
