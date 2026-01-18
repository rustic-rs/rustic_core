use derive_setters::Setters;
use ignore::overrides::{Override, OverrideBuilder};
use serde::{Deserialize, Serialize};

use crate::{ErrorKind, RusticError, RusticResult};

#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Setters, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
/// Options for including/excluding based on globs
pub struct Excludes {
    /// Glob pattern to exclude/include (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long = "glob", value_name = "GLOB"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub globs: Vec<String>,

    /// Same as --glob pattern but ignores the casing of filenames
    #[cfg_attr(feature = "clap", clap(long = "iglob", value_name = "GLOB"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub iglobs: Vec<String>,

    /// Read glob patterns to exclude/include from this file (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long = "glob-file", value_name = "FILE",))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub glob_files: Vec<String>,

    /// Same as --glob-file ignores the casing of filenames in patterns
    #[cfg_attr(feature = "clap", clap(long = "iglob-file", value_name = "FILE",))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub iglob_files: Vec<String>,
}

impl Excludes {
    #[must_use]
    /// Determines if no exclude is in fact given
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub(crate) fn as_override(&self) -> RusticResult<Override> {
        let mut override_builder = OverrideBuilder::new("");
        for g in &self.globs {
            _ = override_builder.add(g).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to add glob pattern `{glob}` to override builder.",
                    err,
                )
                .attach_context("glob", g)
                .ask_report()
            })?;
        }

        for file in &self.glob_files {
            for line in std::fs::read_to_string(file)
                .map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to read string from glob file `{glob_file}` ",
                        err,
                    )
                    .attach_context("glob_file", file)
                    .ask_report()
                })?
                .lines()
            {
                _ = override_builder.add(line).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to add glob pattern line `{glob_pattern_line}` to override builder.",
                        err,
                    )
                    .attach_context("glob_pattern_line", line.to_string())
                    .ask_report()
                })?;
            }
        }

        _ = override_builder.case_insensitive(true).map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to set case insensitivity in override builder.",
                err,
            )
            .ask_report()
        })?;
        for g in &self.iglobs {
            _ = override_builder.add(g).map_err(|err| {
                RusticError::with_source(
                    ErrorKind::Internal,
                    "Failed to add iglob pattern `{iglob}` to override builder.",
                    err,
                )
                .attach_context("iglob", g)
                .ask_report()
            })?;
        }

        for file in &self.iglob_files {
            for line in std::fs::read_to_string(file)
                .map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to read string from iglob file `{iglob_file}`",
                        err,
                    )
                    .attach_context("iglob_file", file)
                    .ask_report()
                })?
                .lines()
            {
                _ = override_builder.add(line).map_err(|err| {
                    RusticError::with_source(
                        ErrorKind::Internal,
                        "Failed to add iglob pattern line `{iglob_pattern_line}` to override builder.",
                        err,
                    )
                    .attach_context("iglob_pattern_line", line.to_string())
                    .ask_report()
                })?;
            }
        }
        let overrides = override_builder.build().map_err(|err| {
            RusticError::with_source(
                ErrorKind::Internal,
                "Failed to build matcher for a set of glob overrides.",
                err,
            )
            .ask_report()
        })?;
        Ok(overrides)
    }
}
