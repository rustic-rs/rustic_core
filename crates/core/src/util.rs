/// Utilities for handling paths on ``rustic_core``
use globset::GlobMatcher;
use serde::{Serialize, Serializer};
use typed_path::{UnixPath, UnixPathBuf};

/// Extend `globset::GlobMatcher` to allow mathing on unix paths (on every platform)
pub trait GlobMatcherExt {
    /// Match on unix paths, i.e. paths which are available as `&[u8]`
    fn is_unix_match(&self, path: impl AsRef<[u8]>) -> bool;
}

impl GlobMatcherExt for GlobMatcher {
    // This is a hacky implementation, espeically for windows where we convert lossily
    // into an utf8 string and match on the windows path given by that string.
    // Note: `GlobMatcher` internally converts into a `&[u8]` to perform the matching
    // TODO: Use https://github.com/BurntSushi/ripgrep/pull/2955 once it is available.
    #[cfg(not(windows))]
    fn is_unix_match(&self, path: impl AsRef<[u8]>) -> bool {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::PathBuf};

        let path = PathBuf::from(OsStr::from_bytes(path.as_ref()));
        self.is_match(&path)
    }
    #[cfg(windows)]
    fn is_unix_match(&self, path: impl AsRef<[u8]>) -> bool {
        use std::{ffi::OsStr, path::Path};

        let string: &str = &String::from_utf8_lossy(path.as_ref());
        let path = Path::new(OsStr::new(string));
        self.is_match(path)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
/// Like `UnixPathBuf` , but implements `Serialize`
pub struct SerializablePath(#[serde(serialize_with = "serialize_unix_path")] pub UnixPathBuf);

fn serialize_unix_path<S>(path: &UnixPath, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = format!("{}", path.display());
    serializer.serialize_str(&s)
}
