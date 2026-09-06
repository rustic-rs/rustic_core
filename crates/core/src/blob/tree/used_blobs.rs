//! Stream-decode restic trees for prune without materializing `Node`s.
//!
//! Prune only needs file content blob ids and directory subtree ids. Full
//! `Tree` deserialize also allocates names, metadata, and xattrs.

use std::{borrow::Cow, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

use crate::blob::{DataId, tree::TreeId};

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
    #[serde(default)]
    content: Option<Vec<DataId>>,
    #[serde(default)]
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
                    if let Some(content) = node.content {
                        self.0.file_blobs.extend(content);
                    }
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
}
