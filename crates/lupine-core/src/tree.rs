//! The tree every device shows. Each node starts at the place of its
//! winning head, then the protocol's rules settle deleted and missing
//! parents, cycles and colliding names, in that order.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::ids::{NodeId, VersionId};
use crate::names::{self, Folded};
use crate::node::{NodeType, NodeVersion};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub node: NodeId,
    /// The winning head, a tombstone when `revived`.
    pub version: VersionId,
    pub kind: EntryKind,
    /// Where the node shows, after the rules.
    pub parent: Option<NodeId>,
    pub name: String,
    /// Rule 1: a deleted folder that a live node inside brings back.
    pub revived: bool,
    /// Rule 2: shown at the root because its parent is unknown or not a
    /// folder, once every drive relay has been synced.
    pub orphan: bool,
    /// Rule 3: moved to the root to break a cycle.
    pub cycle_broken: bool,
    /// Rule 4: suffixed because its name collides with a sibling's.
    pub renamed: bool,
}

impl Entry {
    /// The entry shows a state that no version holds. Orphans keep their
    /// parent and name, which may still arrive.
    pub fn needs_repair(&self) -> bool {
        self.revived || self.cycle_broken || (self.renamed && !self.orphan)
    }
}

pub struct Tree {
    entries: HashMap<NodeId, Entry>,
    children: HashMap<Option<NodeId>, Vec<NodeId>>,
    waiting: Vec<NodeId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Up {
    Root,
    Parent(NodeId),
    Wait,
}

struct Draft<'a> {
    version: VersionId,
    head: &'a NodeVersion,
    kind: EntryKind,
    revived: bool,
}

impl Tree {
    /// `winners` holds the winning head of every node. `complete` once the
    /// client has synced every drive relay.
    pub fn build<'a>(
        winners: impl IntoIterator<Item = (VersionId, &'a NodeVersion)>,
        complete: bool,
    ) -> Self {
        let winners: HashMap<NodeId, (VersionId, &NodeVersion)> = winners
            .into_iter()
            .map(|(id, version)| (version.node, (id, version)))
            .collect();

        let mut drafts: HashMap<NodeId, Draft> = winners
            .iter()
            .filter(|(_, (_, head))| !head.is_tombstone() && head.node_type != NodeType::Share)
            .map(|(&node, &(version, head))| {
                let kind = match head.node_type {
                    NodeType::File => EntryKind::File,
                    _ => EntryKind::Folder,
                };
                let draft = Draft {
                    version,
                    head,
                    kind,
                    revived: false,
                };
                (node, draft)
            })
            .collect();

        // Rule 1: deleted parents of live nodes come back, up the tree.
        let live: Vec<NodeId> = drafts.keys().copied().collect();
        for node in live {
            let mut next = drafts[&node].head.parent;
            while let Some(parent) = next {
                if drafts.contains_key(&parent) {
                    break;
                }
                let Some(&(version, tombstone)) = winners.get(&parent) else {
                    break;
                };
                let draft = Draft {
                    version,
                    head: tombstone,
                    kind: EntryKind::Folder,
                    revived: true,
                };
                drafts.insert(parent, draft);
                next = tombstone.parent;
            }
        }

        // Rule 2: unknown parents, and parents that are not folders.
        let mut orphans = HashSet::new();
        let mut up: HashMap<NodeId, Up> = drafts
            .iter()
            .map(|(&node, draft)| {
                let up = match draft.head.parent {
                    None => Up::Root,
                    Some(parent)
                        if drafts
                            .get(&parent)
                            .is_some_and(|p| p.kind == EntryKind::Folder) =>
                    {
                        Up::Parent(parent)
                    }
                    Some(_) if complete => {
                        orphans.insert(node);
                        Up::Root
                    }
                    Some(_) => Up::Wait,
                };
                (node, up)
            })
            .collect();

        // Rule 3: each loop loses its most recent node to the root.
        let mut broken = HashSet::new();
        let mut done = HashSet::new();
        let mut order: Vec<NodeId> = drafts.keys().copied().collect();
        order.sort();
        for start in order {
            let mut path = Vec::new();
            let mut on_path = HashMap::new();
            let mut current = start;
            loop {
                if done.contains(&current) {
                    break;
                }
                if let Some(&at) = on_path.get(&current) {
                    let breaker = *path[at..]
                        .iter()
                        .max_by_key(|n: &&NodeId| {
                            let draft = &drafts[*n];
                            (draft.head.time, Reverse(draft.version))
                        })
                        .expect("a loop has nodes");
                    up.insert(breaker, Up::Root);
                    broken.insert(breaker);
                    break;
                }
                on_path.insert(current, path.len());
                path.push(current);
                match up[&current] {
                    Up::Parent(parent) => current = parent,
                    Up::Root | Up::Wait => break,
                }
            }
            done.extend(path);
        }

        let mut shown: HashMap<NodeId, bool> = HashMap::new();
        for &node in drafts.keys() {
            let mut path = Vec::new();
            let mut current = node;
            let visible = loop {
                if let Some(&known) = shown.get(&current) {
                    break known;
                }
                path.push(current);
                match up[&current] {
                    Up::Root => break true,
                    Up::Wait => break false,
                    Up::Parent(parent) => current = parent,
                }
            };
            for n in path {
                shown.insert(n, visible);
            }
        }

        let mut children: HashMap<Option<NodeId>, Vec<NodeId>> = HashMap::new();
        let mut waiting = Vec::new();
        for (&node, &visible) in &shown {
            if visible {
                let parent = match up[&node] {
                    Up::Parent(parent) => Some(parent),
                    _ => None,
                };
                children.entry(parent).or_default().push(node);
            } else {
                waiting.push(node);
            }
        }
        waiting.sort();

        // Rule 4: colliding names, per folder.
        let mut entries = HashMap::new();
        for (&parent, siblings) in children.iter_mut() {
            siblings.sort();
            let mut taken = BTreeSet::new();
            let mut losers = Vec::new();
            for &node in siblings.iter() {
                if !taken.insert(Folded(drafts[&node].head.name.clone())) {
                    losers.push(node);
                }
            }
            let mut renamed = HashMap::new();
            for node in losers {
                let name = &drafts[&node].head.name;
                let free = (2..)
                    .map(|n| names::numbered(name, n))
                    .find(|candidate| !taken.contains(&Folded(candidate.clone())))
                    .expect("a folder holds finitely many names");
                taken.insert(Folded(free.clone()));
                renamed.insert(node, free);
            }
            for &node in siblings.iter() {
                let draft = &drafts[&node];
                let new_name = renamed.remove(&node);
                let entry = Entry {
                    node,
                    version: draft.version,
                    kind: draft.kind,
                    parent,
                    renamed: new_name.is_some(),
                    name: new_name.unwrap_or_else(|| draft.head.name.clone()),
                    revived: draft.revived,
                    orphan: orphans.contains(&node),
                    cycle_broken: broken.contains(&node),
                };
                entries.insert(node, entry);
            }
        }

        Self {
            entries,
            children,
            waiting,
        }
    }

    pub fn get(&self, node: &NodeId) -> Option<&Entry> {
        self.entries.get(node)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// `None` lists the drive root. Ordered by node id.
    pub fn children(&self, parent: Option<NodeId>) -> impl Iterator<Item = &Entry> {
        self.children
            .get(&parent)
            .into_iter()
            .flatten()
            .map(|node| &self.entries[node])
    }

    /// Hidden until every drive relay has been synced: the node, or one of
    /// its ancestors, has a parent that is unknown or not a folder.
    pub fn waiting(&self) -> &[NodeId] {
        &self.waiting
    }

    /// Ordered by node id.
    pub fn repairs(&self) -> Vec<&Entry> {
        let mut repairs: Vec<_> = self.entries.values().filter(|e| e.needs_repair()).collect();
        repairs.sort_by_key(|e| e.node);
        repairs
    }

    /// Names from the root down to `node`.
    pub fn path(&self, node: &NodeId) -> Option<Vec<&str>> {
        let mut path = Vec::new();
        let mut current = self.entries.get(node)?;
        loop {
            path.push(current.name.as_str());
            match current.parent {
                Some(parent) => current = &self.entries[&parent],
                None => break,
            }
        }
        path.reverse();
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ContentDescriptor, FileKey};
    use crate::ids::DeviceId;
    use crate::node::Stamp;

    struct Fixture {
        device: DeviceId,
        seq: u64,
        versions: Vec<NodeVersion>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                device: DeviceId::random(),
                seq: 0,
                versions: Vec::new(),
            }
        }

        fn stamp(&mut self) -> Stamp {
            self.seq += 1;
            Stamp {
                device: self.device,
                seq: self.seq,
                epoch: 0,
            }
        }

        fn folder(&mut self, parent: Option<NodeId>, name: &str) -> NodeId {
            let node = NodeId::random();
            let stamp = self.stamp();
            self.versions
                .push(NodeVersion::folder(stamp, node, parent, name));
            node
        }

        fn file(&mut self, parent: Option<NodeId>, name: &str) -> NodeId {
            let node = NodeId::random();
            let content = ContentDescriptor {
                size: 0,
                chunk_size: 1 << 16,
                chunks: Some(Vec::new()),
                table: None,
            };
            let stamp = self.stamp();
            let version =
                NodeVersion::file(stamp, node, parent, name, &FileKey::generate(), content);
            self.versions.push(version);
            node
        }

        fn version(&mut self, node: NodeId) -> &mut NodeVersion {
            self.versions.iter_mut().find(|v| v.node == node).unwrap()
        }

        fn delete(&mut self, node: NodeId) {
            let stamp = self.stamp();
            let live = self.version(node).clone();
            *self.version(node) = NodeVersion::tombstone(stamp, &[&live]).unwrap();
        }

        fn tree(&self, complete: bool) -> Tree {
            let winners = self
                .versions
                .iter()
                .map(|v| (VersionId::of(&serde_json::to_vec(v).unwrap()), v));
            Tree::build(winners, complete)
        }
    }

    fn names(tree: &Tree, parent: Option<NodeId>) -> Vec<String> {
        let mut names: Vec<_> = tree.children(parent).map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn nodes_show_where_their_winning_head_puts_them() {
        let mut f = Fixture::new();
        let photos = f.folder(None, "Photos");
        let trip = f.folder(Some(photos), "Trip");
        let beach = f.file(Some(trip), "beach.jpg");
        let tree = f.tree(false);
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.path(&beach).unwrap(), ["Photos", "Trip", "beach.jpg"]);
        assert_eq!(tree.get(&beach).unwrap().kind, EntryKind::File);
        assert!(tree.repairs().is_empty());
    }

    #[test]
    fn deleted_nodes_disappear() {
        let mut f = Fixture::new();
        let photos = f.folder(None, "Photos");
        f.delete(photos);
        assert!(f.tree(true).is_empty());
    }

    #[test]
    fn a_live_node_revives_its_deleted_ancestors() {
        let mut f = Fixture::new();
        let photos = f.folder(None, "Photos");
        let trip = f.folder(Some(photos), "Trip");
        let beach = f.file(Some(trip), "beach.jpg");
        f.delete(trip);
        f.delete(photos);

        let tree = f.tree(false);
        assert_eq!(tree.path(&beach).unwrap(), ["Photos", "Trip", "beach.jpg"]);
        let revived: Vec<_> = tree.repairs().iter().map(|e| e.node).collect();
        let mut expected = vec![photos, trip];
        expected.sort();
        assert_eq!(revived, expected);
        assert_eq!(tree.get(&trip).unwrap().kind, EntryKind::Folder);
    }

    #[test]
    fn nodes_with_a_missing_parent_wait_for_a_full_sync() {
        let mut f = Fixture::new();
        let missing = NodeId::random();
        let orphan = f.folder(Some(missing), "Orphan");
        let inside = f.file(Some(orphan), "inside.txt");
        f.folder(None, "Orphan");

        let partial = f.tree(false);
        assert_eq!(partial.waiting(), {
            let mut w = vec![orphan, inside];
            w.sort();
            w
        });
        assert_eq!(names(&partial, None), ["Orphan"]);

        let complete = f.tree(true);
        assert!(complete.waiting().is_empty());
        assert_eq!(names(&complete, None), ["Orphan", "Orphan (2)"]);
        let entry = complete.get(&orphan).unwrap();
        assert!(entry.orphan && entry.parent.is_none());
        assert_eq!(complete.path(&inside).unwrap().len(), 2);
        assert!(
            complete.repairs().iter().all(|e| e.node != orphan),
            "orphans are not repaired"
        );
    }

    #[test]
    fn a_file_is_not_a_parent() {
        let mut f = Fixture::new();
        let file = f.file(None, "notes.txt");
        let child = f.folder(Some(file), "Inside");
        let tree = f.tree(true);
        assert!(tree.get(&child).unwrap().orphan);
        assert_eq!(names(&tree, None), ["Inside", "notes.txt"]);
    }

    #[test]
    fn the_most_recent_node_of_a_loop_goes_to_the_root() {
        let mut f = Fixture::new();
        let a = f.folder(None, "A");
        let b = f.folder(Some(a), "B");
        let c = f.folder(Some(b), "C");
        f.version(a).parent = Some(c);
        f.version(a).time = 1;
        f.version(b).time = 3;
        f.version(c).time = 2;

        let tree = f.tree(false);
        assert_eq!(tree.len(), 3);
        let breaker = tree.get(&b).unwrap();
        assert!(breaker.cycle_broken && breaker.parent.is_none());
        assert_eq!(tree.path(&a).unwrap(), ["B", "C", "A"]);
        let repairs: Vec<_> = tree.repairs().iter().map(|e| e.node).collect();
        assert_eq!(repairs, [b]);
    }

    #[test]
    fn a_folder_inside_itself_goes_to_the_root() {
        let mut f = Fixture::new();
        let a = f.folder(None, "A");
        f.version(a).parent = Some(a);
        let tree = f.tree(false);
        assert!(tree.get(&a).unwrap().cycle_broken);
        assert_eq!(names(&tree, None), ["A"]);
    }

    #[test]
    fn colliding_names_get_a_number_in_node_order() {
        let mut f = Fixture::new();
        let folder = f.folder(None, "Docs");
        let nodes: Vec<_> = ["Report.pdf", "report.pdf", "REPORT.PDF", "report (2).pdf"]
            .into_iter()
            .map(|name| f.file(Some(folder), name))
            .collect();
        let tree = f.tree(false);

        let mut sorted = nodes[..3].to_vec();
        sorted.sort();
        let keeper = tree.get(&sorted[0]).unwrap();
        assert!(!keeper.renamed);
        let suffixed: Vec<_> = sorted[1..]
            .iter()
            .map(|n| tree.get(n).unwrap().name.clone())
            .collect();
        let stems: Vec<_> = suffixed
            .iter()
            .map(|n| n.split(' ').nth(1).unwrap().to_lowercase())
            .collect();
        assert_eq!(stems, ["(3).pdf", "(4).pdf"], "(2) is taken by a sibling");
        assert!(!tree.get(&nodes[3]).unwrap().renamed);
        assert_eq!(tree.repairs().len(), 2);
    }

    #[test]
    fn every_device_builds_the_same_tree() {
        let mut f = Fixture::new();
        let root = f.folder(None, "Shared");
        for name in ["x", "X", "x", "y"] {
            f.file(Some(root), name);
        }
        let shown = |tree: Tree| {
            let mut entries: Vec<_> = tree
                .entries()
                .map(|e| (e.node, e.parent, e.name.clone()))
                .collect();
            entries.sort();
            entries
        };
        let first = shown(f.tree(true));
        f.versions.reverse();
        assert_eq!(shown(f.tree(true)), first);
    }
}
