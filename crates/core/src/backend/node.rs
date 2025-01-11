use std::{borrow::Cow, cmp::Ordering, fmt::Debug};

#[cfg(not(windows))]
use std::fmt::Write;
#[cfg(not(windows))]
use std::num::ParseIntError;

use chrono::{DateTime, Local};
use derive_more::Constructor;
use serde_aux::prelude::*;
use serde_derive::{Deserialize, Serialize};
use serde_with::{
    DefaultOnNull,
    base64::{Base64, Standard},
    formats::Padded,
    serde_as, skip_serializing_none,
};
use typed_path::TypedPath;

use crate::blob::{DataId, tree::TreeId};

#[cfg(not(windows))]
/// [`NodeErrorKind`] describes the errors that can be returned by an action utilizing a node in Backends
#[derive(thiserror::Error, Debug, displaydoc::Display)]
#[non_exhaustive]
pub enum NodeErrorKind<'a> {
    /// Unexpected EOF while parsing filename: `{file_name}`
    #[cfg(not(windows))]
    UnexpectedEOF {
        /// The filename
        file_name: String,
        /// The remaining chars
        chars: std::str::Chars<'a>,
    },
    /// Invalid unicode
    #[cfg(not(windows))]
    InvalidUnicode {
        /// The filename
        file_name: String,
        /// The unicode codepoint
        unicode: u32,
        /// The remaining chars
        chars: std::str::Chars<'a>,
    },
    /// Unrecognized Escape while parsing filename: `{file_name}`
    #[cfg(not(windows))]
    UnrecognizedEscape {
        /// The filename
        file_name: String,
        /// The remaining chars
        chars: std::str::Chars<'a>,
    },
    /// Parsing hex chars {chars:?} failed for `{hex}` in filename: `{file_name}` : `{source}`
    #[cfg(not(windows))]
    ParsingHexFailed {
        /// The filename
        file_name: String,
        /// The hex string
        hex: String,
        /// The remaining chars
        chars: std::str::Chars<'a>,
        /// The error that occurred
        source: ParseIntError,
    },
    /// Parsing unicode chars {chars:?} failed for `{target}` in filename: `{file_name}` : `{source}`
    #[cfg(not(windows))]
    ParsingUnicodeFailed {
        /// The filename
        file_name: String,
        /// The target type
        target: String,
        /// The remaining chars
        chars: std::str::Chars<'a>,
        /// The error that occurred
        source: ParseIntError,
    },
}

#[cfg(not(windows))]
pub(crate) type NodeResult<'a, T> = Result<T, NodeErrorKind<'a>>;

#[derive(
    Default, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Constructor, PartialOrd, Ord,
)]
/// A node within the tree hierarchy
pub struct Node {
    /// Name of the node: filename or dirname.
    ///
    /// # Warning
    ///
    /// * This contains an escaped variant of the name in order to handle non-unicode filenames.
    /// * Don't access this field directly, use the [`Node::name()`] method instead!
    pub name: String,
    #[serde(flatten)]
    /// Information about node type
    pub node_type: NodeType,
    #[serde(flatten)]
    /// Node Metadata
    pub meta: Metadata,
    #[serde(default, deserialize_with = "deserialize_default_from_null")]
    /// Contents of the Node
    ///
    /// # Note
    ///
    /// This should be only set for regular files.
    pub content: Option<Vec<DataId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Subtree of the Node.
    ///
    /// # Note
    ///
    /// This should be only set for directories. (TODO: Check if this is correct)
    pub subtree: Option<TreeId>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
#[serde(tag = "type", rename_all = "lowercase")]
/// Types a [`Node`] can have with type-specific additional information
pub enum NodeType {
    /// Node is a regular file
    #[strum(to_string = "file")]
    File,
    /// Node is a directory
    #[strum(to_string = "dir")]
    Dir,
    /// Node is a symlink
    #[strum(to_string = "symlink:{linktarget}")]
    Symlink {
        /// The target of the symlink
        ///
        /// # Warning
        ///
        /// * This contains the target only if it is a valid unicode target.
        /// * Don't access this field directly, use the [`NodeType::to_link()`] method instead!
        linktarget: String,
        #[serde_as(as = "DefaultOnNull<Option<Base64::<Standard,Padded>>>")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// The raw link target saved as bytes.
        ///
        /// This is only filled (and mandatory) if the link target is non-unicode.
        linktarget_raw: Option<Vec<u8>>,
    },
    /// Node is a block device file
    #[strum(to_string = "dev:{device}")]
    Dev {
        #[serde(default)]
        /// Device id
        device: u64,
    },
    /// Node is a char device file
    #[strum(to_string = "chardev:{device}")]
    Chardev {
        #[serde(default)]
        /// Device id
        device: u64,
    },
    /// Node is a fifo
    #[strum(to_string = "fifo")]
    Fifo,
    /// Node is a socket
    #[strum(to_string = "socket")]
    Socket,
}

impl NodeType {
    /// Get a [`NodeType`] from a linktarget path
    #[must_use]
    pub fn from_link(target: &TypedPath<'_>) -> Self {
        let (linktarget, linktarget_raw) = target.to_str().map_or_else(
            || {
                (
                    target.to_string_lossy().to_string(),
                    Some(target.as_bytes().to_vec()),
                )
            },
            |t| (t.to_string(), None),
        );
        Self::Symlink {
            linktarget,
            linktarget_raw,
        }
    }

    // Must be only called on NodeType::Symlink!
    /// Get the link path from a `NodeType::Symlink`.
    ///
    /// # Panics
    ///
    /// * If called on a non-symlink node
    #[must_use]
    pub fn to_link(&self) -> TypedPath<'_> {
        TypedPath::derive(match self {
            Self::Symlink {
                linktarget,
                linktarget_raw,
            } => linktarget_raw
                .as_ref()
                .map_or_else(|| linktarget.as_bytes(), |t| t),
            _ => panic!("called method to_link on non-symlink!"),
        })
    }
}

impl Default for NodeType {
    fn default() -> Self {
        Self::File
    }
}

/// Metadata of a [`Node`]
#[skip_serializing_none]
#[serde_with::apply(
    u64 => #[serde(default, skip_serializing_if = "is_default")],
)]
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Metadata {
    /// Unix file mode
    pub mode: Option<u32>,
    /// Unix mtime (last modification time)
    pub mtime: Option<DateTime<Local>>,
    /// Unix atime (last access time)
    pub atime: Option<DateTime<Local>>,
    /// Unix ctime (last status change time)
    pub ctime: Option<DateTime<Local>>,
    /// Unix uid (user id)
    pub uid: Option<u32>,
    /// Unix gid (group id)
    pub gid: Option<u32>,
    /// Unix user name
    pub user: Option<String>,
    /// Unix group name
    pub group: Option<String>,
    /// Unix inode number
    pub inode: u64,
    /// Unix device id
    pub device_id: u64,
    /// Size of the node
    pub size: u64,
    /// Number of hardlinks to this node
    pub links: u64,
    /// Extended attributes of the node
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extended_attributes: Vec<ExtendedAttribute>,
}

pub(crate) fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    t == &T::default()
}

/// Extended attribute of a [`Node`]
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ExtendedAttribute {
    /// Name of the extended attribute
    pub name: String,
    /// Value of the extended attribute
    #[serde_as(as = "DefaultOnNull<Option<Base64::<Standard,Padded>>>")]
    pub value: Option<Vec<u8>>,
}

impl Node {
    /// Create a new [`Node`] with the given name, type and metadata
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the node
    /// * `node_type` - Type of the node
    /// * `meta` - Metadata of the node
    ///
    /// # Returns
    ///
    /// The created [`Node`]
    #[must_use]
    pub fn new_node(name: &[u8], node_type: NodeType, meta: Metadata) -> Self {
        Self {
            name: escape_filename(name),
            node_type,
            content: None,
            subtree: None,
            meta,
        }
    }
    #[must_use]
    /// Evaluates if this node is a directory
    pub const fn is_dir(&self) -> bool {
        matches!(self.node_type, NodeType::Dir)
    }

    #[must_use]
    /// Evaluates if this node is a symlink
    pub const fn is_symlink(&self) -> bool {
        matches!(self.node_type, NodeType::Symlink { .. })
    }

    #[must_use]
    /// Evaluates if this node is a regular file
    pub const fn is_file(&self) -> bool {
        matches!(self.node_type, NodeType::File)
    }

    #[must_use]
    /// Evaluates if this node is a special file
    pub const fn is_special(&self) -> bool {
        matches!(
            self.node_type,
            NodeType::Symlink { .. }
                | NodeType::Dev { .. }
                | NodeType::Chardev { .. }
                | NodeType::Fifo
                | NodeType::Socket
        )
    }

    #[must_use]
    /// Get the node name as `OsString`, handling name ecaping
    ///
    /// # Panics
    ///
    /// * If the name is not valid unicode
    pub fn name(&self) -> Cow<'_, [u8]> {
        unescape_filename(&self.name).map_or(Cow::Borrowed(self.name.as_bytes()), Cow::Owned)
    }
}

/// An ordering function returning the latest node by mtime
///
/// # Arguments
///
/// * `n1` - First node
/// * `n2` - Second node
///
/// # Returns
///
/// The ordering of the two nodes
#[must_use]
pub fn last_modified_node(n1: &Node, n2: &Node) -> Ordering {
    n1.meta.mtime.cmp(&n2.meta.mtime)
}

/// Escape a filename
///
/// # Arguments
///
/// * `name` - The filename to escape
// This escapes the filename in a way that *should* be compatible to golangs
// stconv.Quote, see https://pkg.go.dev/strconv#Quote
// However, so far there was no specification what Quote really does, so this
// is some kind of try-and-error and maybe does not cover every case.
fn escape_filename(name: &[u8]) -> String {
    let mut input = name;
    let mut s = String::with_capacity(name.len());

    let push = |s: &mut String, p: &str| {
        for c in p.chars() {
            match c {
                '\\' => s.push_str("\\\\"),
                '\"' => s.push_str("\\\""),
                '\u{7}' => s.push_str("\\a"),
                '\u{8}' => s.push_str("\\b"),
                '\u{c}' => s.push_str("\\f"),
                '\n' => s.push_str("\\n"),
                '\r' => s.push_str("\\r"),
                '\t' => s.push_str("\\t"),
                '\u{b}' => s.push_str("\\v"),
                c => s.push(c),
            }
        }
    };

    loop {
        match std::str::from_utf8(input) {
            Ok(valid) => {
                push(&mut s, valid);
                break;
            }
            Err(error) => {
                let (valid, after_valid) = input.split_at(error.valid_up_to());
                push(&mut s, std::str::from_utf8(valid).unwrap());

                if let Some(invalid_sequence_length) = error.error_len() {
                    for b in &after_valid[..invalid_sequence_length] {
                        write!(s, "\\x{b:02x}").unwrap();
                    }
                    input = &after_valid[invalid_sequence_length..];
                } else {
                    for b in after_valid {
                        write!(s, "\\x{b:02x}").unwrap();
                    }
                    break;
                }
            }
        }
    }
    s
}

/// Unescape a filename
///
/// # Arguments
///
/// * `s` - The escaped filename
// inspired by the enquote crate
fn unescape_filename(s: &str) -> NodeResult<'_, Vec<u8>> {
    let mut chars = s.chars();
    let mut u = Vec::new();
    loop {
        match chars.next() {
            None => break,
            Some(c) => {
                if c == '\\' {
                    match chars.next() {
                        None => {
                            return Err(NodeErrorKind::UnexpectedEOF {
                                file_name: s.to_string(),
                                chars,
                            });
                        }
                        Some(c) => match c {
                            '\\' => u.push(b'\\'),
                            '"' => u.push(b'"'),
                            '\'' => u.push(b'\''),
                            '`' => u.push(b'`'),
                            'a' => u.push(b'\x07'),
                            'b' => u.push(b'\x08'),
                            'f' => u.push(b'\x0c'),
                            'n' => u.push(b'\n'),
                            'r' => u.push(b'\r'),
                            't' => u.push(b'\t'),
                            'v' => u.push(b'\x0b'),
                            // hex
                            'x' => {
                                let hex = take(&mut chars, 2);
                                u.push(u8::from_str_radix(&hex, 16).map_err(|err| {
                                    NodeErrorKind::ParsingHexFailed {
                                        file_name: s.to_string(),
                                        hex: hex.to_string(),
                                        chars: chars.clone(),
                                        source: err,
                                    }
                                })?);
                            }
                            // unicode
                            'u' => {
                                let n = u32::from_str_radix(&take(&mut chars, 4), 16).map_err(
                                    |err| NodeErrorKind::ParsingUnicodeFailed {
                                        file_name: s.to_string(),
                                        target: "u32".to_string(),
                                        chars: chars.clone(),
                                        source: err,
                                    },
                                )?;
                                let c = std::char::from_u32(n).ok_or_else(|| {
                                    NodeErrorKind::InvalidUnicode {
                                        file_name: s.to_string(),
                                        unicode: n,
                                        chars: chars.clone(),
                                    }
                                })?;
                                let mut bytes = vec![0u8; c.len_utf8()];
                                _ = c.encode_utf8(&mut bytes);
                                u.extend_from_slice(&bytes);
                            }
                            'U' => {
                                let n = u32::from_str_radix(&take(&mut chars, 8), 16).map_err(
                                    |err| NodeErrorKind::ParsingUnicodeFailed {
                                        file_name: s.to_string(),
                                        target: "u32".to_string(),
                                        chars: chars.clone(),
                                        source: err,
                                    },
                                )?;
                                let c = std::char::from_u32(n).ok_or_else(|| {
                                    NodeErrorKind::InvalidUnicode {
                                        file_name: s.to_string(),
                                        unicode: n,
                                        chars: chars.clone(),
                                    }
                                })?;
                                let mut bytes = vec![0u8; c.len_utf8()];
                                _ = c.encode_utf8(&mut bytes);
                                u.extend_from_slice(&bytes);
                            }
                            _ => {
                                return Err(NodeErrorKind::UnrecognizedEscape {
                                    file_name: s.to_string(),
                                    chars: chars.clone(),
                                });
                            }
                        },
                    }
                } else {
                    let mut bytes = vec![0u8; c.len_utf8()];
                    _ = c.encode_utf8(&mut bytes);
                    u.extend_from_slice(&bytes);
                }
            }
        }
    }

    Ok(u)
}

#[cfg(not(windows))]
#[inline]
// Iterator#take cannot be used because it consumes the iterator
fn take<I: Iterator<Item = char>>(iterator: &mut I, n: usize) -> String {
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        s.push(iterator.next().unwrap_or_default());
    }
    s
}

#[cfg(not(windows))]
#[cfg(test)]
mod tests {
    use super::*;

    use proptest::prelude::*;
    use rstest::rstest;
    use typed_path::UnixPath;

    proptest! {
        #[test]
        fn escape_unescape_is_identity(name in prop::collection::vec(prop::num::u8::ANY, 0..65536)) {
            prop_assert_eq!(unescape_filename(&escape_filename(&name)).unwrap(), name);
        }
    }

    #[rstest]
    #[case(b"\\", r#"\\"#)]
    #[case(b"\"", r#"\""#)]
    #[case(b"'", r#"'"#)]
    #[case(b"`", r#"`"#)]
    #[case(b"\x07", r#"\a"#)]
    #[case(b"\x08", r#"\b"#)]
    #[case(b"\x0b", r#"\v"#)]
    #[case(b"\x0c", r#"\f"#)]
    #[case(b"\n", r#"\n"#)]
    #[case(b"\r", r#"\r"#)]
    #[case(b"\t", r#"\t"#)]
    #[case(b"\xab", r#"\xab"#)]
    #[case(b"\xc2", r#"\xc2"#)]
    #[case(b"\xff", r#"\xff"#)]
    #[case(b"\xc3\x9f", "\u{00df}")]
    #[case(b"\xe2\x9d\xa4", "\u{2764}")]
    #[case(b"\xf0\x9f\x92\xaf", "\u{01f4af}")]
    fn escape_cases(#[case] name: &[u8], #[case] expected: &str) {
        assert_eq!(expected, escape_filename(name));
    }

    #[rstest]
    #[case(r#"\\"#, b"\\")]
    #[case(r#"\""#, b"\"")]
    #[case(r#"\'"#, b"\'")]
    #[case(r#"\`"#, b"`")]
    #[case(r#"\a"#, b"\x07")]
    #[case(r#"\b"#, b"\x08")]
    #[case(r#"\v"#, b"\x0b")]
    #[case(r#"\f"#, b"\x0c")]
    #[case(r#"\n"#, b"\n")]
    #[case(r#"\r"#, b"\r")]
    #[case(r#"\t"#, b"\t")]
    #[case(r#"\xab"#, b"\xab")]
    #[case(r#"\xAB"#, b"\xab")]
    #[case(r#"\xFF"#, b"\xff")]
    #[case(r#"\u00df"#, b"\xc3\x9f")]
    #[case(r#"\u00DF"#, b"\xc3\x9f")]
    #[case(r#"\u2764"#, b"\xe2\x9d\xa4")]
    #[case(r#"\U0001f4af"#, b"\xf0\x9f\x92\xaf")]
    fn unescape_cases(#[case] input: &str, #[case] expected: &[u8]) {
        assert_eq!(expected, unescape_filename(input).unwrap());
    }

    proptest! {
        #[test]
        fn from_link_to_link_is_identity(bytes in prop::collection::vec(prop::num::u8::ANY, 0..65536)) {
            let path = TypedPath::Unix(UnixPath::new(&bytes));
            let node = NodeType::from_link(&path);
            let link = node.to_link();
            prop_assert_eq!(bytes, link.as_bytes());
        }
    }
}
