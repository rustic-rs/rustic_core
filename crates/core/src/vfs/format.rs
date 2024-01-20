use std::fmt;

use crate::repofile::SnapshotFile;

use runtime_format::{FormatKey, FormatKeyError};

pub struct FormattedSnapshot<'a> {
    pub snap: &'a SnapshotFile,
    pub timeformat: &'a str,
}

impl<'a> FormatKey for FormattedSnapshot<'a> {
    fn fmt(&self, key: &str, f: &mut fmt::Formatter<'_>) -> Result<(), FormatKeyError> {
        match key {
            "id" => write!(f, "{}", self.snap.id),
            "long_id" => write!(f, "{:?}", self.snap.id),
            "time" => write!(f, "{}", self.snap.time.format(self.timeformat)),
            "username" => write!(f, "{}", self.snap.username),
            "hostname" => write!(f, "{}", self.snap.hostname),
            "label" => write!(f, "{}", self.snap.label),
            "tags" => write!(f, "{}", self.snap.tags),
            "backup_start" => {
                if let Some(summary) = &self.snap.summary {
                    write!(f, "{}", summary.backup_start.format(self.timeformat))
                } else {
                    write!(f, "no_backup_start")
                }
            }
            "backup_end" => {
                if let Some(summary) = &self.snap.summary {
                    write!(f, "{}", summary.backup_end.format(self.timeformat))
                } else {
                    write!(f, "no_backup_end")
                }
            }

            _ => return Err(FormatKeyError::UnknownKey),
        }
        .map_err(FormatKeyError::Fmt)
    }
}
