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
    /// Glob pattern to include/exclude (can be specified multiple times).
    ///
    /// A pattern without `!` includes only matching paths. To exclude matching
    /// paths while including all other paths, prefix it with `!`, for example
    /// `!**/*.tmp`.
    #[cfg_attr(feature = "clap", clap(long = "glob", value_name = "GLOB"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub globs: Vec<String>,

    /// Same as `--glob`, but ignores the casing of filenames.
    ///
    /// A pattern without `!` includes only matching paths. To exclude matching
    /// paths while including all other paths, prefix it with `!`.
    #[cfg_attr(feature = "clap", clap(long = "iglob", value_name = "GLOB"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub iglobs: Vec<String>,

    /// Read `--glob` patterns from this file (can be specified multiple times).
    ///
    /// Each line has the same semantics as `--glob`: a pattern without `!`
    /// includes only matching paths, while `!pattern` excludes matching paths.
    #[cfg_attr(feature = "clap", clap(long = "glob-file", value_name = "FILE",))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub glob_files: Vec<String>,

    /// Same as `--glob-file`, but ignores the casing of filenames in patterns.
    ///
    /// Each line has the same semantics as `--iglob`.
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

    /// Convert excludes to an Override for glob pattern matching
    ///
    /// # Errors
    ///
    /// * If glob patterns cannot be compiled into an override
    pub fn as_override(&self) -> RusticResult<Override> {
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

#[cfg(test)]
mod tests {
    use super::Excludes;

    #[test]
    fn glob_patterns_use_override_include_exclude_semantics() {
        let includes = Excludes::default()
            .globs(vec!["**/*.tmp".to_owned()])
            .as_override()
            .expect("glob should compile");
        assert!(includes.matched("dir/file.tmp", false).is_whitelist());
        assert!(includes.matched("dir/file.txt", false).is_ignore());

        let excludes = Excludes::default()
            .globs(vec!["!**/*.tmp".to_owned()])
            .as_override()
            .expect("glob should compile");
        assert!(excludes.matched("dir/file.tmp", false).is_ignore());
        assert!(excludes.matched("dir/file.txt", false).is_none());
    }
}
