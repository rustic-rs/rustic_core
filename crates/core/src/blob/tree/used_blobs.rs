//! Stream-decode restic trees for prune without materializing `Node`s.
//!
//! Prune only needs file content blob ids and directory subtree ids. A
//! dedicated JSON scanner skips names, metadata, and xattrs without UTF-8
//! validation or serde parse of unused fields.

use std::borrow::Cow;

use memchr::memchr2;
use serde::de::Error as DeError;

use crate::{
    Id,
    blob::{DataId, tree::TreeId},
};

/// Nibble lookup: 0–15 for hex digits, `0xFF` otherwise.
const fn hex_nibble_table() -> [u8; 256] {
    let mut t = [0xff_u8; 256];
    let mut i: u8 = 0;
    while i < 10 {
        t[b'0' as usize + i as usize] = i;
        i += 1;
    }
    i = 0;
    while i < 6 {
        t[b'a' as usize + i as usize] = 10 + i;
        t[b'A' as usize + i as usize] = 10 + i;
        i += 1;
    }
    t
}

const HEX_NIBBLE: [u8; 256] = hex_nibble_table();

/// Index of the closing `"` in a JSON string body (after the opening quote).
fn find_unescaped_quote(bytes: &[u8]) -> Result<usize, ScanError> {
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        match memchr2(b'"', b'\\', &bytes[i..]) {
            None => break,
            Some(rel) => {
                i += rel;
                match bytes[i] {
                    b'"' => return Ok(i),
                    b'\\' => {
                        if i + 1 >= n {
                            return Scan::err("unterminated string escape");
                        }
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
        }
    }
    Scan::err("unterminated string")
}

#[inline]
fn decode_hex32(src: &[u8]) -> Option<[u8; 32]> {
    if src.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    let mut i = 0;
    while i < 32 {
        let hi = HEX_NIBBLE[src[i * 2] as usize];
        let lo = HEX_NIBBLE[src[i * 2 + 1] as usize];
        if (hi | lo) == 0xff {
            return None;
        }
        out[i] = (hi << 4) | lo;
        i += 1;
    }
    Some(out)
}

type ScanError = serde_json::Error;

/// Compact tree contents used by prune's used-blob walk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UsedBlobsTree {
    pub file_blobs: Vec<DataId>,
    pub dir_trees: Vec<TreeId>,
}

#[derive(Debug, Default, Clone, Copy)]
enum UsedBlobKind {
    File,
    Dir,
    #[default]
    Other,
}

struct Scan<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Scan<'a> {
    fn err<T>(msg: &'static str) -> Result<T, ScanError> {
        Err(DeError::custom(msg))
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    #[inline]
    fn bump(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.bump();
        }
    }

    #[inline]
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), ScanError> {
        if self.eat(c) {
            Ok(())
        } else {
            Self::err("unexpected JSON token")
        }
    }

    fn skip_lit(&mut self, lit: &[u8]) -> Result<(), ScanError> {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        if rest.starts_with(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Self::err("invalid JSON literal")
        }
    }

    fn try_null(&mut self) -> Result<bool, ScanError> {
        if self.peek() == Some(b'n') {
            self.skip_lit(b"null")?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Skip the rest of a JSON string. `pos` is already past the opening quote.
    fn skip_string_body(&mut self) -> Result<(), ScanError> {
        let bytes = self.buf.get(self.pos..).unwrap_or(&[]);
        let i = find_unescaped_quote(bytes)?;
        self.pos += i + 1;
        Ok(())
    }

    fn skip_string(&mut self) -> Result<(), ScanError> {
        if !self.eat(b'"') {
            return Self::err("expected string");
        }
        self.skip_string_body()
    }

    fn skip_digits(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.bump();
        }
        self.pos > start
    }

    fn skip_number(&mut self) -> Result<(), ScanError> {
        let _ = self.eat(b'-');
        if !self.skip_digits() {
            return Self::err("invalid number");
        }
        if self.eat(b'.') && !self.skip_digits() {
            return Self::err("invalid number");
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            if !self.skip_digits() {
                return Self::err("invalid number");
            }
        }
        Ok(())
    }

    fn skip_value(&mut self) -> Result<(), ScanError> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => self.skip_string(),
            Some(b'{') => self.skip_comma_list(b'{', b'}', true),
            Some(b'[') => self.skip_comma_list(b'[', b']', false),
            Some(b't') => self.skip_lit(b"true"),
            Some(b'f') => self.skip_lit(b"false"),
            Some(b'n') => self.skip_lit(b"null"),
            Some(b'-' | b'0'..=b'9') => self.skip_number(),
            _ => Self::err("expected JSON value"),
        }
    }

    fn skip_comma_list(&mut self, open: u8, close: u8, object: bool) -> Result<(), ScanError> {
        self.expect(open)?;
        let mut first = true;
        loop {
            self.skip_ws();
            if self.eat(close) {
                return Ok(());
            }
            if !first {
                self.expect(b',')?;
                self.skip_ws();
                if self.eat(close) {
                    return Self::err("trailing comma");
                }
            }
            first = false;
            if object {
                self.skip_string()?;
                self.skip_ws();
                self.expect(b':')?;
            }
            self.skip_value()?;
        }
    }

    /// Borrow ordinary ASCII keys and type names; decode JSON escapes only
    /// on the slow path so escaped live references keep their meaning.
    fn parse_short_string(&mut self) -> Result<Cow<'a, [u8]>, ScanError> {
        self.expect(b'"')?;
        let start = self.pos;
        let bytes = &self.buf[start..];
        for (i, byte) in bytes.iter().copied().enumerate() {
            match byte {
                b'"' => {
                    self.pos = start + i + 1;
                    return Ok(Cow::Borrowed(&self.buf[start..start + i]));
                }
                b'\\' | 0x80..=0xff => {
                    self.pos = start + i;
                    self.skip_string_body()?;
                    let decoded: String = serde_json::from_slice(&self.buf[start - 1..self.pos])?;
                    return Ok(Cow::Owned(decoded.into_bytes()));
                }
                0..=0x1f => return Self::err("control character in JSON string"),
                _ => {}
            }
        }
        Self::err("unterminated string")
    }

    fn parse_key(&mut self) -> Result<Cow<'a, [u8]>, ScanError> {
        self.parse_short_string()
    }

    fn parse_kind(&mut self) -> Result<UsedBlobKind, ScanError> {
        self.skip_ws();
        Ok(match self.parse_short_string()?.as_ref() {
            b"file" => UsedBlobKind::File,
            b"dir" => UsedBlobKind::Dir,
            b"symlink" | b"dev" | b"chardev" | b"fifo" | b"socket" => UsedBlobKind::Other,
            _ => return Self::err("unknown node type"),
        })
    }

    fn parse_hex_id(&mut self) -> Result<Id, ScanError> {
        self.skip_ws();
        if !self.eat(b'"') {
            return Self::err("expected hex id string");
        }
        let start = self.pos - 1;
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        if rest.len() >= 65
            && rest[64] == b'"'
            && let Some(bytes) = decode_hex32(&rest[..64])
        {
            self.pos += 65;
            return Ok(Id::new(bytes));
        }
        self.skip_string_body()?;
        let decoded: String = serde_json::from_slice(&self.buf[start..self.pos])?;
        decode_hex32(decoded.as_bytes())
            .map(Id::new)
            .ok_or_else(|| DeError::custom("invalid hex blob id"))
    }

    fn parse_content(&mut self, tree: &mut UsedBlobsTree) -> Result<(), ScanError> {
        self.skip_ws();
        if self.try_null()? {
            return Ok(());
        }
        self.expect(b'[')?;
        let mut first = true;
        loop {
            self.skip_ws();
            if self.eat(b']') {
                return Ok(());
            }
            if !first {
                self.expect(b',')?;
                self.skip_ws();
                if self.eat(b']') {
                    return Self::err("trailing comma");
                }
            }
            first = false;
            tree.file_blobs.push(DataId::from(self.parse_hex_id()?));
        }
    }

    fn parse_subtree(&mut self, tree: &mut UsedBlobsTree) -> Result<(), ScanError> {
        self.skip_ws();
        if self.try_null()? {
            return Ok(());
        }
        tree.dir_trees.push(TreeId::from(self.parse_hex_id()?));
        Ok(())
    }

    fn parse_node(&mut self, tree: &mut UsedBlobsTree) -> Result<(), ScanError> {
        self.skip_ws();
        self.expect(b'{')?;
        let files_at = tree.file_blobs.len();
        let dirs_at = tree.dir_trees.len();
        let mut kind = None;
        let mut first = true;
        loop {
            self.skip_ws();
            if self.eat(b'}') {
                break;
            }
            if !first {
                self.expect(b',')?;
                self.skip_ws();
                if self.eat(b'}') {
                    return Self::err("trailing comma");
                }
            }
            first = false;
            let key = self.parse_key()?;
            self.skip_ws();
            self.expect(b':')?;
            match key.as_ref() {
                b"type" => {
                    if kind.is_some() {
                        return Self::err("duplicate node type");
                    }
                    kind = Some(self.parse_kind()?);
                }
                b"content" => self.parse_content(tree)?,
                b"subtree" => self.parse_subtree(tree)?,
                _ => self.skip_value()?,
            }
        }
        match kind.ok_or_else(|| <ScanError as DeError>::custom("missing node type"))? {
            UsedBlobKind::File => tree.dir_trees.truncate(dirs_at),
            UsedBlobKind::Dir => tree.file_blobs.truncate(files_at),
            UsedBlobKind::Other => {
                tree.file_blobs.truncate(files_at);
                tree.dir_trees.truncate(dirs_at);
            }
        }
        Ok(())
    }

    fn parse_nodes(&mut self, tree: &mut UsedBlobsTree) -> Result<(), ScanError> {
        self.skip_ws();
        if self.try_null()? {
            return Ok(());
        }
        self.expect(b'[')?;
        let mut first = true;
        loop {
            self.skip_ws();
            if self.eat(b']') {
                return Ok(());
            }
            if !first {
                self.expect(b',')?;
                self.skip_ws();
                if self.eat(b']') {
                    return Self::err("trailing comma");
                }
            }
            first = false;
            self.parse_node(tree)?;
        }
    }

    fn parse_tree(&mut self) -> Result<UsedBlobsTree, ScanError> {
        self.skip_ws();
        self.expect(b'{')?;
        let mut tree = UsedBlobsTree::default();
        let mut first = true;
        loop {
            self.skip_ws();
            if self.eat(b'}') {
                break;
            }
            if !first {
                self.expect(b',')?;
                self.skip_ws();
                if self.eat(b'}') {
                    return Self::err("trailing comma");
                }
            }
            first = false;
            let key = self.parse_key()?;
            self.skip_ws();
            self.expect(b':')?;
            if key.as_ref() == b"nodes" {
                self.parse_nodes(&mut tree)?;
            } else {
                self.skip_value()?;
            }
        }
        self.skip_ws();
        if self.pos != self.buf.len() {
            return Self::err("trailing JSON");
        }
        Ok(tree)
    }
}

pub(crate) fn parse_used_blobs_tree(data: &[u8]) -> Result<UsedBlobsTree, serde_json::Error> {
    Scan { buf: data, pos: 0 }.parse_tree()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::node::Node;
    use crate::blob::tree::Tree;

    const FILE_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const TREE_ID: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    #[test]
    fn extracts_file_and_dir_ids_and_skips_the_rest() {
        let json = format!(
            r#"{{
                "nodes": [
                    {{
                        "name": "foo",
                        "type": "file",
                        "mtime": "2020-01-01T00:00:00+00:00",
                        "mode": 420,
                        "uid": 1000,
                        "user": "brad",
                        "inode": 1,
                        "size": 3,
                        "links": 1,
                        "extended_attributes": [{{"name": "user.foo", "value": "YQ=="}}],
                        "content": ["{FILE_ID}"]
                    }},
                    {{
                        "name": "bar",
                        "type": "dir",
                        "subtree": "{TREE_ID}"
                    }},
                    {{
                        "name": "link",
                        "type": "symlink",
                        "linktarget": "/tmp/x"
                    }}
                ]
            }}"#
        );

        let used = parse_used_blobs_tree(json.as_bytes()).unwrap();
        let full: Tree = serde_json::from_slice(json.as_bytes()).unwrap();

        let full_files: Vec<_> = full
            .nodes
            .iter()
            .filter(|n| matches!(n.node_type, crate::backend::node::NodeType::File))
            .flat_map(|n| n.content.iter().flatten().copied())
            .collect();
        let full_dirs: Vec<_> = full.nodes.iter().filter_map(|n| n.subtree).collect();

        assert_eq!(used.file_blobs, full_files);
        assert_eq!(used.dir_trees, full_dirs);
        assert_eq!(used.file_blobs, vec![FILE_ID.parse::<DataId>().unwrap()]);
        assert_eq!(used.dir_trees, vec![TREE_ID.parse::<TreeId>().unwrap()]);
        // Full deserialize kept metadata we did not allocate in the prune path.
        let foo: &Node = &full.nodes[0];
        assert_eq!(foo.name, "foo");
        assert_eq!(foo.meta.extended_attributes.len(), 1);
    }

    #[test]
    fn null_or_missing_nodes_is_empty() {
        assert_eq!(
            parse_used_blobs_tree(br#"{"nodes":null}"#).unwrap(),
            UsedBlobsTree::default()
        );
        assert_eq!(
            parse_used_blobs_tree(br#"{}"#).unwrap(),
            UsedBlobsTree::default()
        );
        assert_eq!(
            parse_used_blobs_tree(br#"{"nodes":[]}"#).unwrap(),
            UsedBlobsTree::default()
        );
    }

    #[test]
    fn ignores_unknown_tree_keys() {
        let json = format!(r#"{{"extra":1,"nodes":[{{"type":"file","content":["{FILE_ID}"]}}]}}"#);
        let used = parse_used_blobs_tree(json.as_bytes()).unwrap();
        assert_eq!(used.file_blobs.len(), 1);
        assert!(used.dir_trees.is_empty());
    }

    #[test]
    fn hex_ids_accept_uppercase_and_reject_garbage() {
        let upper = format!(
            r#"{{"nodes":[{{"type":"file","content":["{}"]}}]}}"#,
            FILE_ID.to_uppercase()
        );
        assert_eq!(
            parse_used_blobs_tree(upper.as_bytes()).unwrap().file_blobs,
            vec![FILE_ID.parse::<DataId>().unwrap()]
        );
        assert!(
            parse_used_blobs_tree(br#"{"nodes":[{"type":"file","content":["zzzz"]}]}"#).is_err()
        );
    }

    #[test]
    fn skips_escaped_names_and_accepts_content_before_type() {
        let json =
            format!(r#"{{"nodes":[{{"name":"quo\"te","content":["{FILE_ID}"],"type":"file"}}]}}"#);
        let used = parse_used_blobs_tree(json.as_bytes()).unwrap();
        assert_eq!(used.file_blobs, vec![FILE_ID.parse::<DataId>().unwrap()]);
        assert!(used.dir_trees.is_empty());
    }

    #[test]
    fn file_content_is_ignored_on_dirs_and_other_types() {
        let json = format!(
            r#"{{"nodes":[{{"type":"dir","content":["{FILE_ID}"],"subtree":"{TREE_ID}"}},{{"type":"symlink","content":["{FILE_ID}"]}}]}}"#
        );
        let used = parse_used_blobs_tree(json.as_bytes()).unwrap();
        assert!(used.file_blobs.is_empty());
        assert_eq!(used.dir_trees, vec![TREE_ID.parse::<TreeId>().unwrap()]);
    }

    #[test]
    fn escaped_live_references_match_full_tree() {
        let json = format!(
            r#"{{"nodes":[{{"name":"file","type":"file","content":["{FILE_ID}"]}},{{"name":"dir","type":"dir","subtree":"{TREE_ID}"}}]}}"#
        );
        for (plain, escaped) in [
            (r#""nodes""#, r#""n\u006fdes""#),
            (r#""type""#, r#""t\u0079pe""#),
            (r#""content""#, r#""cont\u0065nt""#),
            (r#""subtree""#, r#""subtr\u0065e""#),
            (r#""file""#, r#""f\u0069le""#),
            (r#""dir""#, r#""d\u0069r""#),
            ("012345", r"\u003012345"),
            ("fedcba", r"\u0066edcba"),
        ] {
            let escaped_json = json.replace(plain, escaped);
            let full: Tree = serde_json::from_str(&escaped_json).unwrap();
            let used = parse_used_blobs_tree(escaped_json.as_bytes()).unwrap();
            let files: Vec<_> = full
                .nodes
                .iter()
                .flat_map(|n| n.content.iter().flatten().copied())
                .collect();
            let dirs: Vec<_> = full.nodes.iter().filter_map(|n| n.subtree).collect();
            assert_eq!(used.file_blobs, files, "{escaped_json}");
            assert_eq!(used.dir_trees, dirs, "{escaped_json}");
        }
    }

    #[test]
    fn rejects_ambiguous_node_types_and_invalid_escapes() {
        for json in [
            r#"{"nodes":[{"type":"future_file"}]}"#,
            r#"{"nodes":[{"name":"missing type"}]}"#,
            r#"{"nodes":[{"type":"file","type":"symlink"}]}"#,
            r#"{"n\qodes":[]}"#,
            r#"{"nodes":[{"type":"f\qile"}]}"#,
            r#"{"nodes":[{"type":"file","content":["\q"]}]}"#,
        ] {
            assert!(parse_used_blobs_tree(json.as_bytes()).is_err(), "{json}");
        }
    }
    #[test]
    fn find_unescaped_quote_handles_escapes_and_long_bodies() {
        assert_eq!(find_unescaped_quote(b"\"").unwrap(), 0);
        assert_eq!(find_unescaped_quote(b"hello\"").unwrap(), 5);
        assert_eq!(find_unescaped_quote(br#"quo\"te""#).unwrap(), 7);
        assert_eq!(find_unescaped_quote(br#"foo\\""#).unwrap(), 5);
        let long = [b'a'; 80];
        let mut body = long.to_vec();
        body.push(b'"');
        assert_eq!(find_unescaped_quote(&body).unwrap(), 80);
        assert!(find_unescaped_quote(b"noend").is_err());
        assert!(find_unescaped_quote(b"abc\\").is_err());
    }

    #[test]
    fn skips_long_unescaped_names() {
        let name = "n".repeat(80);
        let json = format!(
            r#"{{"nodes":[{{"name":"{name}","mtime":"2020-01-01T00:00:00+00:00","type":"file","content":["{FILE_ID}"]}}]}}"#
        );
        let used = parse_used_blobs_tree(json.as_bytes()).unwrap();
        assert_eq!(used.file_blobs, vec![FILE_ID.parse::<DataId>().unwrap()]);
    }
}
