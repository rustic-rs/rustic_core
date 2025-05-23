/// Utilities for handling paths on ``rustic_core``
///
use std::{
    borrow::Cow,
    ffi::OsStr,
    path::{Path, PathBuf},
    str::Utf8Error,
};

use serde::{Serialize, Serializer};
use typed_path::{
    Component, TypedPath, UnixComponent, UnixPath, UnixPathBuf, WindowsComponent, WindowsPath,
    WindowsPrefix,
};

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

/// Converts a [`Path`] to a [`WindowsPath`].
///
/// # Arguments
///
/// * `path` - The path to convert.
///
/// # Errors
///
/// * If the path is non-unicode
pub fn path_to_windows_path(path: &Path) -> Result<&WindowsPath, Utf8Error> {
    let str = std::str::from_utf8(path.as_os_str().as_encoded_bytes())?;
    Ok(WindowsPath::new(str))
}

/// Converts a [`Path`] to a [`Cow<UnixPath>`].
///
/// Note: On windows, this converts prefixes into unix paths, e.g. "C:\dir" into "/c/dir"
///
/// # Arguments
///
/// * `path` - The path to convert.
///
/// # Errors
///
/// * If the path is non-unicode and we are using windows
pub fn path_to_unix_path(path: &Path) -> Result<Cow<'_, UnixPath>, Utf8Error> {
    #[cfg(not(windows))]
    {
        let path = UnixPath::new(path.as_os_str().as_encoded_bytes());
        Ok(Cow::Borrowed(path))
    }
    #[cfg(windows)]
    {
        let path = windows_path_to_unix_path(path_to_windows_path(path)?);
        Ok(Cow::Owned(path))
    }
}

/// Converts a [`TypedPath`] to a [`Cow<Path>`].
///
/// Note: On unix, this converts windows prefixes into unix paths, e.g. "C:\dir" into "/c/dir"
///
/// # Arguments
///
/// * `path` - The path to convert.
///
/// # Errors
///
/// * If the path is non-unicode and we are using windows
pub fn typed_path_to_path<'a>(path: &'a TypedPath<'a>) -> Result<Cow<'a, Path>, Utf8Error> {
    #[cfg(not(windows))]
    {
        let path = match typed_path_to_unix_path(path) {
            Cow::Borrowed(path) => Cow::Borrowed(unix_path_to_path(path)?),
            Cow::Owned(path) => Cow::Owned(unix_path_to_path(&path)?.to_path_buf()),
        };
        Ok(path)
    }
    #[cfg(windows)]
    {
        // only utf8 items are allowed on windows
        let str = std::str::from_utf8(path.as_bytes())?;
        Ok(Cow::Borrowed(Path::new(str)))
    }
}

/// Converts a [`UnixPath`] to a [`Path`].
///
/// # Arguments
///
/// * `path` - The path to convert.
///
/// # Errors
///
/// * If the path is non-unicode and we are using windows
pub fn unix_path_to_path(path: &UnixPath) -> Result<&Path, Utf8Error> {
    #[cfg(not(windows))]
    {
        let osstr: &OsStr = path.as_ref();
        Ok(Path::new(osstr))
    }
    #[cfg(windows)]
    {
        // only utf8 items are allowed on windows
        let str = std::str::from_utf8(path.as_bytes())?;
        Ok(Path::new(str))
    }
}

/// Converts a [`[u8]`] to a [`PathBuf`].
// This is a hacky implementation, espeically for windows where we convert lossily
// into an utf8 string and match on the windows path given by that string.
// Note: `GlobMatcher` internally converts into a `&[u8]` to perform the matching
// TODO: Use https://github.com/BurntSushi/ripgrep/pull/2955 once it is available.
#[cfg(not(windows))]
pub fn u8_to_path(path: impl AsRef<[u8]>) -> PathBuf {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    Path::new(OsStr::from_bytes(path.as_ref())).to_path_buf()
}
#[cfg(windows)]
pub fn u8_to_path(&self, path: impl AsRef<[u8]>) -> PathBuf {
    use std::ffi::OsStr;
    let string: &str = &String::from_utf8_lossy(path.as_ref());
    Path::new(OsStr::new(string)).to_path_buf()
}

/// Converts a [`TypedPath`] to a [`Cow<UnixPath>`].
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
/// Note: This converts windows prefixes into unix paths, e.g. "C:\dir" into "/c/dir"
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
                        unix_path.push(p.to_ascii_lowercase());
                    }
                    WindowsPrefix::VerbatimUNC(_, q) | WindowsPrefix::UNC(_, q) => {
                        unix_path.push(q.to_ascii_lowercase());
                    }
                    WindowsPrefix::VerbatimDisk(p) | WindowsPrefix::Disk(p) => {
                        let c = vec![p.to_ascii_lowercase()];
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
    #[case(r#"C:\"#, "/c")]
    #[case(r#"C:\dir"#, "/c/dir")]
    #[case(r#"d:\"#, "/d")]
    #[case(r#"a\b\"#, "a/b")]
    #[case(r#"a\b\c"#, "a/b/c")]
    fn test_windows_path_to_unix_path(#[case] windows_path: &str, #[case] unix_path: &str) {
        assert_eq!(
            windows_path_to_unix_path(WindowsPath::new(windows_path))
                .to_str()
                .unwrap(),
            UnixPath::new(unix_path).to_str().unwrap()
        );
    }
}
