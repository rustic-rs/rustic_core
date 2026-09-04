//! Stream-decode restic trees for prune without materializing `Node`s.
//!
//! Prune only needs file content blob ids and directory subtree ids. Full
//! `Tree` deserialize also allocates names, metadata, and xattrs.

use std::{borrow::Cow, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

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

fn parse_hex_id(s: &str) -> Option<Id> {
    decode_hex32(s.as_bytes()).map(Id::new)
}

struct HexDataIdVisitor;

impl Visitor<'_> for HexDataIdVisitor {
    type Value = DataId;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a 64-character hex blob id")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        parse_hex_id(v)
            .map(DataId::from)
            .ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
    }
}

struct HexTreeIdVisitor;

impl Visitor<'_> for HexTreeIdVisitor {
    type Value = TreeId;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a 64-character hex tree id")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        parse_hex_id(v)
            .map(TreeId::from)
            .ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
    }
}

fn deserialize_data_ids<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<DataId>, D::Error> {
    struct SeqVisitor;

    impl<'de> Visitor<'de> for SeqVisitor {
        type Value = Vec<DataId>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array of hex blob ids")
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_seq(self)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(id) = seq.next_element_seed(HexDataIdSeed)? {
                out.push(id);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(SeqVisitor)
}

struct HexDataIdSeed;

impl<'de> DeserializeSeed<'de> for HexDataIdSeed {
    type Value = DataId;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(HexDataIdVisitor)
    }
}

fn deserialize_opt_tree_id<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TreeId>, D::Error> {
    struct OptVisitor;

    impl<'de> Visitor<'de> for OptVisitor {
        type Value = Option<TreeId>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hex tree id or null")
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_str(HexTreeIdVisitor).map(Some)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            HexTreeIdVisitor.visit_str(v).map(Some)
        }
    }

    deserializer.deserialize_any(OptVisitor)
}

/// Compact tree contents used by prune's used-blob walk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UsedBlobsTree {
    pub file_blobs: Vec<DataId>,
    pub dir_trees: Vec<TreeId>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum UsedBlobKind {
    File,
    Dir,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct UsedBlobNode {
    #[serde(rename = "type")]
    kind: UsedBlobKind,
    #[serde(default, deserialize_with = "deserialize_data_ids")]
    content: Vec<DataId>,
    #[serde(default, deserialize_with = "deserialize_opt_tree_id")]
    subtree: Option<TreeId>,
}

impl<'de> Deserialize<'de> for UsedBlobsTree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UsedBlobsTreeVisitor)
    }
}

struct UsedBlobsTreeVisitor;

impl<'de> Visitor<'de> for UsedBlobsTreeVisitor {
    type Value = UsedBlobsTree;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a restic tree object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut tree = UsedBlobsTree::default();
        while let Some(key) = map.next_key::<Cow<'_, str>>()? {
            if key == "nodes" {
                map.next_value_seed(NodesSeed(&mut tree))?;
            } else {
                let _: IgnoredAny = map.next_value()?;
            }
        }
        Ok(tree)
    }
}

struct NodesSeed<'a>(&'a mut UsedBlobsTree);

impl<'de> DeserializeSeed<'de> for NodesSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for NodesSeed<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a nodes array or null")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while let Some(node) = seq.next_element::<UsedBlobNode>()? {
            match node.kind {
                UsedBlobKind::File => {
                    self.0.file_blobs.extend(node.content);
                }
                UsedBlobKind::Dir => {
                    if let Some(subtree) = node.subtree {
                        self.0.dir_trees.push(subtree);
                    }
                }
                UsedBlobKind::Other => {}
            }
        }
        Ok(())
    }
}

pub(crate) fn parse_used_blobs_tree(data: &[u8]) -> Result<UsedBlobsTree, serde_json::Error> {
    serde_json::from_slice(data)
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
}
