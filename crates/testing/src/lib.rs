//! Testing utilities for the `rustic` ecosystem.

// formatting args are used for error messages
#![allow(clippy::literal_string_with_formatting_args)]

/// Backends to be used solely for testing.
pub mod backend;

use aho_corasick::{AhoCorasick, PatternID};
use std::{error::Error, ffi::OsStr};
use tempfile::NamedTempFile;

/// A test result.
pub type TestResult<T> = Result<T, Box<dyn Error>>;

/// Get the matches for the given patterns and output.
///
/// # Arguments
///
/// * `patterns` - The patterns to search for.
/// * `output` - The output to search in.
///
/// # Errors
///
/// If the patterns are invalid.
///
/// # Returns
///
/// The matches for the given patterns and output.
pub fn get_matches<I, P>(patterns: I, output: &str) -> TestResult<Vec<(PatternID, usize)>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<[u8]>,
{
    let ac = AhoCorasick::new(patterns)?;
    let mut matches = vec![];
    for mat in ac.find_iter(output) {
        add_match_to_vector(&mut matches, mat);
    }
    Ok(matches)
}

/// Add a match to the given vector.
///
/// # Arguments
///
/// * `matches` - The vector to add the match to.
/// * `mat` - The `aho_corasick::Match` to add.
pub fn add_match_to_vector(matches: &mut Vec<(PatternID, usize)>, mat: aho_corasick::Match) {
    matches.push((mat.pattern(), mat.end() - mat.start()));
}

/// Get a temporary file.
///
/// # Errors
///
/// If the temporary file could not be created.
///
/// # Returns
///
/// A temporary file.
pub fn get_temp_file() -> TestResult<NamedTempFile> {
    Ok(NamedTempFile::new()?)
}

/// Check if the given files differ.
///
/// # Arguments
///
/// * `path_left` - The left file to compare.
/// * `path_right` - The right file to compare.
///
/// # Errors
///
/// If the files could not be compared.
///
/// # Returns
///
/// `true` if the files differ, `false` otherwise.
pub fn files_differ(
    path_left: impl AsRef<OsStr>,
    path_right: impl AsRef<OsStr>,
) -> TestResult<bool> {
    // diff the directories
    #[cfg(not(windows))]
    {
        let proc = std::process::Command::new("diff")
            .arg(path_left)
            .arg(path_right)
            .output()?;

        if proc.stdout.is_empty() {
            return Ok(false);
        }
    }

    #[cfg(windows)]
    {
        let proc = std::process::Command::new("fc.exe")
            .arg("/L")
            .arg(path_left)
            .arg(path_right)
            .output()?;

        let output = String::from_utf8(proc.stdout)?;

        dbg!(&output);

        let patterns = &["FC: no differences encountered"];
        let ac = AhoCorasick::new(patterns)?;
        let mut matches = vec![];

        for mat in ac.find_iter(output.as_str()) {
            matches.push((mat.pattern(), mat.end() - mat.start()));
        }

        if matches == vec![(PatternID::must(0), 30)] {
            return Ok(false);
        }
    }

    Ok(true)
}
