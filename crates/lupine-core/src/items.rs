//! The tree as file system integrations show it: one item per entry, and
//! what changed from one tree to the next.

use std::cmp::Reverse;
use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::ids::NodeId;
use crate::node::FileMeta;
use crate::replica::Replica;
use crate::tree::{EntryKind, Tree};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub node: NodeId,
    /// `None` at the drive root.
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: EntryKind,
    /// Unix milliseconds.
    pub created: Option<i64>,
    pub modified: Option<i64>,
    /// Files only.
    pub file: Option<FileMeta>,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.kind == EntryKind::Folder
    }

    pub fn size(&self) -> u64 {
        self.file.as_ref().map_or(0, |file| file.content.size)
    }

    /// Changes with the content, and only then. `None` for folders.
    pub fn content_version(&self) -> Option<[u8; 32]> {
        let file = self.file.as_ref()?;
        let json = serde_json::to_vec(file).expect("file metadata serializes");
        Some(Sha256::digest(json).into())
    }

    /// Changes with anything shown but the content.
    pub fn metadata_version(&self) -> [u8; 32] {
        let shown = (
            self.parent,
            &self.name,
            self.is_folder(),
            self.created,
            self.modified,
        );
        let json = serde_json::to_vec(&shown).expect("item metadata serializes");
        Sha256::digest(json).into()
    }
}

/// Every entry of `tree`, as an item.
pub(crate) fn shown(replica: &Replica, tree: &Tree) -> HashMap<NodeId, Item> {
    tree.entries()
        .map(|entry| {
            let head = replica.version(&entry.version);
            let item = Item {
                node: entry.node,
                parent: entry.parent,
                name: entry.name.clone(),
                kind: entry.kind,
                created: head.and_then(|v| v.created),
                modified: head.and_then(|v| v.modified),
                file: head
                    .filter(|_| entry.kind == EntryKind::File)
                    .and_then(|v| v.file.clone()),
            };
            (entry.node, item)
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct ItemChanges {
    /// Parents before their children.
    pub updated: Vec<Item>,
    /// Children before their parents.
    pub removed: Vec<NodeId>,
}

impl ItemChanges {
    pub fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.removed.is_empty()
    }
}

pub(crate) fn diff(old: &HashMap<NodeId, Item>, new: &HashMap<NodeId, Item>) -> ItemChanges {
    let mut updated: Vec<&Item> = new
        .values()
        .filter(|item| old.get(&item.node) != Some(item))
        .collect();
    updated.sort_by_key(|item| (depth(new, &item.node), item.node));
    let mut removed: Vec<NodeId> = old
        .keys()
        .filter(|node| !new.contains_key(node))
        .copied()
        .collect();
    removed.sort_by_key(|node| (Reverse(depth(old, node)), *node));
    ItemChanges {
        updated: updated.into_iter().cloned().collect(),
        removed,
    }
}

fn depth(items: &HashMap<NodeId, Item>, node: &NodeId) -> usize {
    let mut depth = 0;
    let mut current = items.get(node).and_then(|item| item.parent);
    // A tree has no loops; the bound only guards against a corrupt one.
    while let Some(parent) = current
        && depth < items.len()
    {
        depth += 1;
        current = items.get(&parent).and_then(|item| item.parent);
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ContentDescriptor, FileKey};

    fn folder(parent: Option<NodeId>, name: &str) -> Item {
        Item {
            node: NodeId::random(),
            parent,
            name: name.into(),
            kind: EntryKind::Folder,
            created: None,
            modified: None,
            file: None,
        }
    }

    fn file(parent: Option<NodeId>, name: &str, byte: u8) -> Item {
        let content = ContentDescriptor {
            size: 1,
            chunk_size: 1 << 16,
            chunks: Some(vec![byte; 64]),
            table: None,
        };
        Item {
            kind: EntryKind::File,
            file: Some(FileMeta {
                key: FileKey::generate().root().clone(),
                content,
            }),
            ..folder(parent, name)
        }
    }

    fn map(items: &[&Item]) -> HashMap<NodeId, Item> {
        items.iter().map(|i| (i.node, (*i).clone())).collect()
    }

    #[test]
    fn versions_separate_content_from_metadata() {
        let item = file(None, "a.txt", 1);
        let mut renamed = item.clone();
        renamed.name = "b.txt".into();
        assert_eq!(renamed.content_version(), item.content_version());
        assert_ne!(renamed.metadata_version(), item.metadata_version());

        let mut edited = item.clone();
        edited.file = file(None, "a.txt", 2).file;
        assert_ne!(edited.content_version(), item.content_version());
        assert_eq!(edited.metadata_version(), item.metadata_version());
        assert!(folder(None, "A").content_version().is_none());
    }

    #[test]
    fn changes_come_parents_first_and_removals_children_first() {
        let a = folder(None, "A");
        let b = folder(Some(a.node), "B");
        let c = file(Some(b.node), "c.txt", 1);
        let new = map(&[&c, &b, &a]);
        let changes = diff(&HashMap::new(), &new);
        let order: Vec<_> = changes.updated.iter().map(|i| i.node).collect();
        assert_eq!(order, [a.node, b.node, c.node]);
        assert!(diff(&new, &new).is_empty());

        let removed = diff(&new, &HashMap::new()).removed;
        assert_eq!(removed, [c.node, b.node, a.node]);
    }
}
