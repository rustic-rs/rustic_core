/// Utilities for handling paths on ``rustic_core``
use std::borrow::Cow;

use globset::GlobMatcher;
use serde::{Serialize, Serializer};
use typed_path::{
    Component, TypedPath, UnixComponent, UnixPath, UnixPathBuf, WindowsComponent, WindowsPath,
    WindowsPrefix,
};

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

/// Converts a [`TypedPath`] to an [`Cow<UnixPath>`].
///
/// # Arguments
///
/// * `path` - The path to convert.
#[must_use]
pub fn typed_path_to_unix_path<'a>(path: &'a TypedPath<'_>) -> Cow<'a, UnixPath> {
    match path {
        TypedPath::Unix(p) => Cow::Borrowed(p),
        TypedPath::Windows(p) => Cow::Owned(windows_path_to_unix_path(p)),
    }
}

/// Converts a [`WindowsPath`] to a [`UnixPathBuf`].
///
/// # Arguments
///
/// * `path` - The path to convert.
#[must_use]
pub fn windows_path_to_unix_path(path: &WindowsPath) -> UnixPathBuf {
    let mut unix_path = UnixPathBuf::new();
    let mut components = path.components();
    if let Some(c) = components.next() {
        match c {
            WindowsComponent::Prefix(p) => {
                unix_path.push(UnixComponent::RootDir);
                match p.kind() {
                    WindowsPrefix::Verbatim(p) | WindowsPrefix::DeviceNS(p) => {
                        unix_path.push(p);
                    }
                    WindowsPrefix::VerbatimUNC(_, q) | WindowsPrefix::UNC(_, q) => {
                        unix_path.push(q);
                    }
                    WindowsPrefix::VerbatimDisk(p) | WindowsPrefix::Disk(p) => {
                        let c = vec![p];
                        unix_path.push(&c);
                    }
                }
                // remove RootDir from iterator
                _ = components.next();
            }
            WindowsComponent::RootDir => {
                unix_path.push(UnixComponent::RootDir);
            }
            c => {
                unix_path.push(c.as_bytes());
            }
        }
    }
    for c in components {
        match c {
            WindowsComponent::RootDir => {
                unix_path.push(UnixComponent::RootDir);
            }
            c => {
                unix_path.push(c);
            }
        }
    }
    unix_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    #[rstest]
    #[case("/", "/")]
    #[case(r#"\"#, "/")]
    #[case("/test/test2", "/test/test2")]
    #[case(r#"\test\test2"#, "/test/test2")]
    #[case(r#"C:\"#, "/C")]
    #[case(r#"C:\dir"#, "/C/dir")]
    #[case(r#"a\b\"#, "a/b")]
    #[case(r#"a\b\c"#, "a/b/c")]
    fn test_typed_path_to_unix_path(#[case] windows_path: &str, #[case] unix_path: &str) {
        assert_eq!(
            windows_path_to_unix_path(WindowsPath::new(windows_path))
                .to_str()
                .unwrap(),
            UnixPath::new(unix_path).to_str().unwrap()
        );
    }
}
