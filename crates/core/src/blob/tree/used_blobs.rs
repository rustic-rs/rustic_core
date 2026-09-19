//! Deserialize restic trees for prune without materializing full `Node`s.
//!
//! Prune only needs file content blob ids and directory subtree ids. A
//! dedicated serde struct keeps `type` / `content` / `subtree` and ignores
//! names, metadata, and xattrs.

use serde_derive::Deserialize;

use crate::blob::{DataId, tree::TreeId};

/// Compact tree contents used by prune's used-blob walk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UsedBlobsTree {
    pub file_blobs: Vec<DataId>,
    pub dir_trees: Vec<TreeId>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PruneNodeKind {
    File,
    Dir,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct PruneNode {
    #[serde(rename = "type", default)]
    kind: PruneNodeKind,
    #[serde(default)]
    content: Option<Vec<DataId>>,
    #[serde(default)]
    subtree: Option<TreeId>,
}

#[derive(Debug, Default, Deserialize)]
struct PruneTree {
    #[serde(default, deserialize_with = "super::deserialize_null_default")]
    nodes: Vec<PruneNode>,
}

pub(crate) fn parse_used_blobs_tree(data: &[u8]) -> Result<UsedBlobsTree, serde_json::Error> {
    let parsed: PruneTree = serde_json::from_slice(data)?;
    let mut tree = UsedBlobsTree::default();
    for node in parsed.nodes {
        match node.kind {
            PruneNodeKind::File => {
                if let Some(content) = node.content {
                    tree.file_blobs.extend(content);
                }
            }
            PruneNodeKind::Dir => {
                if let Some(id) = node.subtree {
                    tree.dir_trees.push(id);
                }
            }
            PruneNodeKind::Other => {}
        }
    }
    Ok(tree)
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
    fn unknown_or_missing_types_are_skipped() {
        assert_eq!(
            parse_used_blobs_tree(br#"{"nodes":[{"type":"future_file"}]}"#).unwrap(),
            UsedBlobsTree::default()
        );
        assert_eq!(
            parse_used_blobs_tree(br#"{"nodes":[{"name":"missing type"}]}"#).unwrap(),
            UsedBlobsTree::default()
        );
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_used_blobs_tree(br#"{"n\qodes":[]}"#).is_err());
        assert!(parse_used_blobs_tree(br#"{"nodes":[{"type":"f\qile"}]}"#).is_err());
        assert!(parse_used_blobs_tree(
            br#"{"nodes":[{"type":"file","content":["\q"]}]}"#
        )
        .is_err());
        assert!(parse_used_blobs_tree(
            br#"{"nodes":[{"type":"file","type":"symlink"}]}"#
        )
        .is_err());
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
