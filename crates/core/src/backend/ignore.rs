pub mod mapper;
pub use mapper::LocalSourceSaveOptions;

use std::{
    ffi::OsString,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use bytesize::ByteSize;
use derive_setters::Setters;
use ignore::{DirEntry, Walk, WalkBuilder};
use log::warn;
use serde_with::{DisplayFromStr, serde_as};

#[cfg(not(windows))]
use std::num::TryFromIntError;

use crate::{
    Excludes,
    backend::{
        ReadSource, ReadSourceEntry, ReadSourceOpen,
        cache::{release_cached_open_files, retry_on_too_many_open_files},
    },
    error::{ErrorKind, RusticError, RusticResult, io_error_is_too_many_open_files},
};

/// [`IgnoreErrorKind`] describes the errors that can be returned by a Ignore action in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum IgnoreErrorKind {
    #[cfg(all(not(windows), not(target_os = "openbsd")))]
    /// Error getting xattrs for `{path:?}`: `{source:?}`
    ErrorXattr { path: PathBuf, source: io::Error },
    /// Error reading link target for `{path:?}`: `{source:?}`
    ErrorLink { path: PathBuf, source: io::Error },
    #[cfg(not(windows))]
    /// Error converting ctime `{ctime}` and `ctime_nsec` `{ctime_nsec}` to Utc Timestamp: `{source:?}`
    CtimeConversionToTimestampFailed {
        ctime: i64,
        ctime_nsec: i64,
        source: TryFromIntError,
    },
    /// Error acquiring metadata for `{name}`: `{source:?}`
    AcquiringMetadataFailed { name: String, source: ignore::Error },
    /// time error
    JiffError(#[from] jiff::Error),
}

pub(crate) type IgnoreResult<T> = Result<T, IgnoreErrorKind>;

/// Nested walks spawned after EMFILE on a directory. Pack FDs are dropped first;
/// if `opendir` still fails after this many retries, the backup aborts.
const MAX_EMFILE_RETRIES: u8 = 2;

/// A [`LocalSource`] is a source from local paths which is used to be read from (i.e. to backup it).
#[derive(Debug)]
pub struct LocalSource {
    /// The walk builder.
    builder: WalkBuilder,
    /// The save options to use.
    save_opts: LocalSourceSaveOptions,
    /// Glob excludes, kept so an EMFILE retry can re-walk a single directory.
    excludes: Excludes,
    /// Filter options, kept so an EMFILE retry can re-walk a single directory.
    filter_opts: LocalSourceFilterOptions,
}

#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(serde::Deserialize, serde::Serialize, Default, Clone, Debug, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into)]
#[non_exhaustive]
/// [`LocalSourceFilterOptions`] allow to filter a local source by various criteria.
pub struct LocalSourceFilterOptions {
    /// Ignore files based on .gitignore files
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub git_ignore: bool,

    /// Do not require a git repository to apply git-ignore rule
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub no_require_git: bool,

    /// Treat the provided filename like a .gitignore file (can be specified multiple times)
    #[cfg_attr(
        feature = "clap",
        clap(long = "custom-ignorefile", value_name = "FILE")
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub custom_ignorefiles: Vec<String>,

    /// Exclude contents of directories containing this filename (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long, value_name = "FILE"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub exclude_if_present: Vec<String>,

    /// Exclude files/directories having the given extended attribute set (can be specified multiple times)
    #[cfg_attr(feature = "clap", clap(long, value_name = "XATTR"))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::vec::overwrite_empty))]
    pub exclude_if_xattr: Vec<String>,

    /// Exclude other file systems, don't cross filesystem boundaries and subvolumes
    #[cfg_attr(feature = "clap", clap(long, short = 'x'))]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::bool::overwrite_false))]
    pub one_file_system: bool,

    /// Maximum size of files to be backed up. Larger files will be excluded.
    #[cfg_attr(feature = "clap", clap(long, value_name = "SIZE"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    pub exclude_larger_than: Option<ByteSize>,
}

fn local_walk_builder(
    excludes: &Excludes,
    filter_opts: &LocalSourceFilterOptions,
    backup_paths: &[impl AsRef<Path>],
) -> RusticResult<WalkBuilder> {
    let mut walk_builder = WalkBuilder::new(&backup_paths[0]);

    for path in &backup_paths[1..] {
        _ = walk_builder.add(path);
    }

    let overrides = excludes.as_override()?;

    for file in &filter_opts.custom_ignorefiles {
        _ = walk_builder.add_custom_ignore_filename(file);
    }

    _ = walk_builder
        .follow_links(false)
        .hidden(false)
        .ignore(false)
        .git_ignore(filter_opts.git_ignore)
        .git_exclude(filter_opts.git_ignore)
        .require_git(!filter_opts.no_require_git)
        .sort_by_file_path(Path::cmp)
        .same_file_system(filter_opts.one_file_system)
        .max_filesize(filter_opts.exclude_larger_than.map(|s| s.as_u64()))
        .overrides(overrides);

    let exclude_if_present = filter_opts.exclude_if_present.clone();
    let exclude_if_xattr: Vec<OsString> = filter_opts
        .exclude_if_xattr
        .iter()
        .map(OsString::from)
        .collect();

    if !exclude_if_xattr.is_empty() {
        #[cfg(any(windows, target_os = "openbsd"))]
        warn!("exclude-if-xattr is not supported on this platform");
        #[cfg(not(any(windows, target_os = "openbsd")))]
        if !xattr::SUPPORTED_PLATFORM {
            warn!("exclude-if-xattr is not supported on this platform");
        }
    }

    let needs_entry_filter = !exclude_if_present.is_empty() || !exclude_if_xattr.is_empty();

    if needs_entry_filter {
        _ = walk_builder.filter_entry(move |entry| {
            if !exclude_if_present.is_empty()
                && let Some(tpe) = entry.file_type()
                && tpe.is_dir()
                && exclude_if_present
                    .iter()
                    .any(|file| entry.path().join(file).exists())
            {
                return false;
            }

            #[cfg(not(any(windows, target_os = "openbsd")))]
            if xattr::SUPPORTED_PLATFORM && !exclude_if_xattr.is_empty() {
                match xattr::list(entry.path()) {
                    Ok(mut attrs) => {
                        if attrs.any(|attr| exclude_if_xattr.contains(&attr)) {
                            return false;
                        }
                    }
                    Err(err) => {
                        warn!(
                            "Error reading xattrs for {}, not excluding: {err}",
                            entry.path().display()
                        );
                    }
                }
            }

            true
        });
    }

    Ok(walk_builder)
}

impl LocalSource {
    /// Create a local source from [`LocalSourceSaveOptions`], [`LocalSourceFilterOptions`] and backup path(s).
    ///
    /// # Arguments
    ///
    /// * `save_opts` - The [`LocalSourceSaveOptions`] to use.
    /// * `filter_opts` - The [`LocalSourceFilterOptions`] to use.
    /// * `backup_paths` - The backup path(s) to use.
    ///
    /// # Returns
    ///
    /// The created local source.
    ///
    /// # Errors
    ///
    /// * If the a glob pattern could not be added to the override builder.
    /// * If a glob file could not be read.
    pub fn new(
        save_opts: LocalSourceSaveOptions,
        excludes: &Excludes,
        filter_opts: &LocalSourceFilterOptions,
        backup_paths: &[impl AsRef<Path>],
    ) -> RusticResult<Self> {
        let builder = local_walk_builder(excludes, filter_opts, backup_paths)?;
        Ok(Self {
            builder,
            save_opts,
            excludes: excludes.clone(),
            filter_opts: filter_opts.clone(),
        })
    }
}

#[derive(Debug)]
/// Describes an open file from the local backend.
pub struct OpenFile(PathBuf);

impl ReadSourceOpen for OpenFile {
    type Reader = File;

    /// Open the file from the local backend.
    ///
    /// # Returns
    ///
    /// The read handle to the file from the local backend.
    ///
    /// # Errors
    ///
    /// * If the file could not be opened.
    fn open(self) -> RusticResult<Self::Reader> {
        let path = self.0;
        retry_on_too_many_open_files(|| File::open(&path)).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to open file at `{path}`. Please make sure the file exists and is accessible.",
                err,
            )
            .attach_context("path", path.display().to_string())
        })
    }
}

impl ReadSource for LocalSource {
    type Open = OpenFile;
    type Iter = LocalSourceWalker;

    /// Get the size of the local source.
    ///
    /// # Returns
    ///
    /// The size of the local source or `None` if the size could not be determined.
    ///
    /// # Errors
    ///
    /// * If the size could not be determined.
    fn size(&self) -> RusticResult<Option<u64>> {
        let mut size = 0;
        for entry in self.builder.build() {
            if let Err(err) = entry.and_then(|e| e.metadata()).map(|m| {
                size += if m.is_dir() { 0 } else { m.len() };
            }) {
                if ignore_error_is_too_many_open_files(&err) {
                    release_cached_open_files();
                }
                warn!("ignoring error {err}");
            }
        }
        Ok(Some(size))
    }

    /// Iterate over the entries of the local source.
    ///
    /// # Returns
    ///
    /// An iterator over the entries of the local source.
    fn entries(&self) -> Self::Iter {
        LocalSourceWalker {
            walkers: vec![self.builder.build()],
            save_opts: self.save_opts,
            excludes: self.excludes.clone(),
            filter_opts: self.filter_opts.clone(),
            emfile_retries: 0,
        }
    }
}

fn ignore_error_path(err: &ignore::Error) -> Option<String> {
    ignore_error_path_buf(err).map(|path| path.display().to_string())
}

fn ignore_error_path_buf(err: &ignore::Error) -> Option<PathBuf> {
    match err {
        ignore::Error::WithPath { path, .. } => Some(path.clone()),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            ignore_error_path_buf(err)
        }
        ignore::Error::Loop { child, .. } => Some(child.clone()),
        _ => None,
    }
}

fn ignore_error_is_too_many_open_files(err: &ignore::Error) -> bool {
    err.io_error().is_some_and(io_error_is_too_many_open_files)
}

fn clone_io_error(err: &io::Error) -> io::Error {
    crate::error::io_error_emfile_os_code(err).map_or_else(
        || io::Error::new(err.kind(), err.to_string()),
        io::Error::from_raw_os_error,
    )
}

fn wrap_ignore_error(err: ignore::Error) -> Box<RusticError> {
    let path = ignore_error_path(&err);
    let guidance = if path.is_some() {
        "Failed to read source path `{path}`."
    } else {
        "Failed to read source path."
    };
    // `ignore::Error` does not expose the inner I/O error via `Error::source`.
    let io_source = err.io_error().map(clone_io_error);
    let rustic_err = io_source.map_or_else(
        || RusticError::with_source(ErrorKind::InputOutput, guidance, err),
        |io_err| RusticError::with_source(ErrorKind::InputOutput, guidance, io_err),
    );
    match path {
        Some(path) => rustic_err.attach_context("path", path),
        None => rustic_err,
    }
}

// Walk doesn't implement Debug
#[allow(missing_debug_implementations)]
pub struct LocalSourceWalker {
    /// Walk stack. Extra walks are pushed after EMFILE on a directory.
    walkers: Vec<Walk>,
    /// The save options to use.
    save_opts: LocalSourceSaveOptions,
    excludes: Excludes,
    filter_opts: LocalSourceFilterOptions,
    emfile_retries: u8,
}

impl LocalSourceWalker {
    fn next_raw_entry(&mut self) -> Option<Result<DirEntry, ignore::Error>> {
        loop {
            let walker = self.walkers.last_mut()?;
            match walker.next() {
                None => {
                    _ = self.walkers.pop();
                    self.emfile_retries = self.emfile_retries.saturating_sub(1);
                }
                some => return some,
            }
        }
    }

    fn spawn_nested_walk(&mut self, path: &Path) -> RusticResult<()> {
        let walk = local_walk_builder(&self.excludes, &self.filter_opts, &[path])?.build();
        self.walkers.push(walk);
        self.emfile_retries += 1;
        Ok(())
    }

    fn map_dir_entry(&self, entry: DirEntry) -> RusticResult<ReadSourceEntry<OpenFile>> {
        let path = entry.path().display().to_string();
        self.save_opts.map_entry(entry).map_err(|err| {
            RusticError::with_source(
                ErrorKind::InputOutput,
                "Failed to map directory entry `{path}` to a backup source entry.",
                err,
            )
            .attach_context("path", path)
        })
    }
}

impl Iterator for LocalSourceWalker {
    type Item = RusticResult<ReadSourceEntry<OpenFile>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.next_raw_entry()? {
                // ignore root dir of each walk (backup root, or a retried directory already yielded)
                Ok(entry)
                    if entry.depth() == 0 && entry.file_type().is_some_and(|t| t.is_dir()) => {}
                Ok(entry) => return Some(self.map_dir_entry(entry)),
                Err(err) if ignore_error_is_too_many_open_files(&err) => {
                    let Some(path) = ignore_error_path_buf(&err) else {
                        return Some(Err(wrap_ignore_error(err)));
                    };
                    if self.emfile_retries >= MAX_EMFILE_RETRIES {
                        return Some(Err(wrap_ignore_error(err).prepend_guidance_line(
                            "Too many open files after dropping cached pack FDs; aborting so this snapshot is not missing directories.",
                        )));
                    }
                    warn!(
                        "too many open files reading `{}`, dropping cached pack FDs and retrying",
                        path.display()
                    );
                    release_cached_open_files();
                    if let Err(build_err) = self.spawn_nested_walk(&path) {
                        return Some(Err(build_err));
                    }
                }
                Err(err) => return Some(Err(wrap_ignore_error(err))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io};

    use crate::error::io_error_is_too_many_open_files;

    fn emfile() -> io::Error {
        #[cfg(windows)]
        {
            io::Error::from_raw_os_error(4)
        }
        #[cfg(not(windows))]
        {
            io::Error::from_raw_os_error(24)
        }
    }

    fn local_source(path: &Path) -> LocalSource {
        LocalSource::new(
            LocalSourceSaveOptions::default(),
            &Excludes::default(),
            &LocalSourceFilterOptions::default(),
            &[path],
        )
        .unwrap()
    }

    fn entry_names(src: &LocalSource) -> RusticResult<Vec<String>> {
        src.entries()
            .map(|entry| entry.map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn wrap_ignore_emfile_is_too_many_open_files() {
        let err = ignore::Error::WithPath {
            path: PathBuf::from("/tmp/day"),
            err: Box::new(ignore::Error::from(emfile())),
        };
        let rustic = wrap_ignore_error(err);
        assert!(rustic.is_too_many_open_files());
        assert_eq!(rustic.context_value("path"), Some("/tmp/day"));
    }

    #[test]
    fn rustic_error_from_io_emfile_is_too_many_open_files() {
        let err = RusticError::with_source(ErrorKind::InputOutput, "open", emfile());
        assert!(err.is_too_many_open_files());
        let err = RusticError::with_source(
            ErrorKind::InputOutput,
            "vanished",
            io::Error::from_raw_os_error(2),
        );
        assert!(!err.is_too_many_open_files());
    }

    #[test]
    fn walk_lists_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2021-07-01");
        fs::create_dir(&day).unwrap();
        fs::write(day.join("a.jpg"), b"x").unwrap();
        let names = entry_names(&local_source(dir.path())).unwrap();
        assert!(names.iter().any(|n| n == "2021-07-01"), "{names:?}");
        assert!(names.iter().any(|n| n == "a.jpg"), "{names:?}");
    }

    #[cfg(unix)]
    fn run_in_child(env: &str, exact: &str) -> bool {
        if std::env::var_os(env).is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", exact, "--nocapture", "--test-threads=1"])
            .env(env, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    #[cfg(unix)]
    fn set_nofile(soft: u64) {
        use nix::sys::resource::{Resource, getrlimit, setrlimit};
        let (_, hard) = getrlimit(Resource::RLIMIT_NOFILE).unwrap();
        setrlimit(Resource::RLIMIT_NOFILE, soft.min(hard), hard).unwrap();
    }

    #[cfg(unix)]
    fn open_until_emfile() -> Vec<File> {
        let mut files = Vec::new();
        loop {
            match File::open("/dev/null") {
                Ok(f) => files.push(f),
                Err(err) if io_error_is_too_many_open_files(&err) => break,
                Err(err) => panic!("unexpected open error: {err}"),
            }
        }
        files
    }

    #[cfg(unix)]
    fn photo_tree() -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let mut expected = Vec::new();
        for day in ["2021-07-01", "2021-07-23", "2021-08-27"] {
            let path = dir.path().join(day);
            fs::create_dir(&path).unwrap();
            expected.push(day.to_string());
            let file = format!("{day}.jpg");
            fs::write(path.join(&file), b"x").unwrap();
            expected.push(file);
        }
        (dir, expected)
    }

    #[cfg(unix)]
    #[test]
    fn walk_emfile_is_fatal_not_a_skipped_directory() {
        if !run_in_child(
            "RUSTIC_WALK_EMFILE_FATAL",
            "backend::ignore::tests::walk_emfile_is_fatal_not_a_skipped_directory",
        ) {
            return;
        }

        let (tree, _) = photo_tree();
        let src = local_source(tree.path());
        set_nofile(64);
        let extra = open_until_emfile();

        let err = entry_names(&src).expect_err("EMFILE must not skip dirs and succeed");
        assert!(
            err.is_too_many_open_files(),
            "expected EMFILE, got {}",
            err.display_log()
        );
        drop(extra);
    }
}
