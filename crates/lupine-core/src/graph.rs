//! The versions of one node and the graph their version vectors draw: which
//! versions are heads, which head wins, and what resolving a conflict writes.

use std::cmp::Reverse;

use crate::error::Result;
use crate::ids::{NodeId, VersionId};
use crate::names;
use crate::node::{DeviceClock, NodeVersion};

pub struct History<'a> {
    versions: Vec<(VersionId, &'a NodeVersion)>,
}

impl<'a> History<'a> {
    /// Deduplicates by version id.
    pub fn new(versions: impl IntoIterator<Item = (VersionId, &'a NodeVersion)>) -> Self {
        let mut versions: Vec<_> = versions.into_iter().collect();
        versions.sort_by_key(|(id, _)| *id);
        versions.dedup_by_key(|(id, _)| *id);
        Self { versions }
    }

    pub fn versions(&self) -> &[(VersionId, &'a NodeVersion)] {
        &self.versions
    }

    /// The versions no other version dominates. `None` for an empty history.
    pub fn heads(&self) -> Option<Heads<'a>> {
        let mut heads: Vec<_> = self
            .versions
            .iter()
            .filter(|(_, v)| !self.versions.iter().any(|(_, w)| w.dominates(v)))
            .copied()
            .collect();
        // Only forged vectors can dominate each other in a loop.
        if heads.is_empty() {
            heads.clone_from(&self.versions);
        }
        (!heads.is_empty()).then_some(Heads(heads))
    }
}

/// Never empty.
#[derive(Clone)]
pub struct Heads<'a>(Vec<(VersionId, &'a NodeVersion)>);

impl<'a> Heads<'a> {
    pub fn iter(&self) -> impl Iterator<Item = (VersionId, &'a NodeVersion)> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// What a new version of the node replaces.
    pub fn versions(&self) -> Vec<&'a NodeVersion> {
        self.0.iter().map(|(_, v)| *v).collect()
    }

    /// The most recent live head (greatest `time`, then lowest version id),
    /// or the most recent tombstone when every head is one.
    pub fn winner(&self) -> (VersionId, &'a NodeVersion) {
        *self
            .0
            .iter()
            .max_by_key(|(id, v)| (!v.is_tombstone(), v.time, Reverse(*id)))
            .expect("heads are never empty")
    }

    /// The winner and the heads that hold its state: what a device showing
    /// the node has seen. An edit made there replaces these and leaves any
    /// other head in conflict, so its content becomes a conflict copy
    /// instead of being lost.
    pub fn seen(&self) -> Vec<&'a NodeVersion> {
        let (_, winner) = self.winner();
        self.0
            .iter()
            .map(|(_, head)| *head)
            .filter(|head| head.same_state(winner))
            .collect()
    }

    /// Heads that differ only in who wrote them and when are no conflict.
    pub fn in_conflict(&self) -> bool {
        let (_, winner) = self.winner();
        self.0.iter().any(|(_, head)| !head.same_state(winner))
    }

    /// A new file next to the winner for each live head whose content
    /// differs from the winner's. Ids and names derive from the head, so
    /// devices that resolve the same conflict create the same copies.
    pub fn conflict_copies(&self, clock: &mut DeviceClock, epoch: u32) -> Result<Vec<NodeVersion>> {
        let (winner_id, winner) = self.winner();
        self.0
            .iter()
            .filter(|(id, head)| {
                *id != winner_id && head.file.is_some() && head.file != winner.file
            })
            .map(|(id, head)| {
                let mut copy = (*head).clone();
                copy.node = NodeId::conflict_copy(id);
                copy.name = names::conflict_copy(&head.name, head.time);
                copy.parent = winner.parent;
                copy.rewritten(clock.stamp(epoch), &[])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ContentDescriptor, FileKey};
    use crate::ids::DeviceId;
    use crate::node::{NodeType, Stamp};

    fn stamp(device: DeviceId, seq: u64) -> Stamp {
        Stamp {
            device,
            seq,
            epoch: 0,
        }
    }

    fn file(device: DeviceId, seq: u64, node: NodeId, name: &str, time: i64) -> NodeVersion {
        let content = ContentDescriptor {
            size: 1,
            chunk_size: 1 << 16,
            chunks: Some(vec![seq as u8; 64]),
            table: None,
        };
        let mut version = NodeVersion::file(
            stamp(device, seq),
            node,
            None,
            name,
            &FileKey::generate(),
            content,
        );
        version.time = time;
        version
    }

    fn ids(versions: &[NodeVersion]) -> Vec<(VersionId, &NodeVersion)> {
        versions
            .iter()
            .map(|v| (VersionId::of(&serde_json::to_vec(v).unwrap()), v))
            .collect()
    }

    fn head_names(versions: &[NodeVersion]) -> Vec<String> {
        let history = History::new(ids(versions));
        let mut names: Vec<_> = history
            .heads()
            .unwrap()
            .iter()
            .map(|(_, v)| v.name.clone())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn compacted_versions_that_come_back_stay_superseded() {
        let (a, b, c) = (DeviceId::random(), DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let v1 = NodeVersion::folder(stamp(a, 1), node, None, "One");
        let v2 = NodeVersion::folder(stamp(b, 2), node, None, "Two")
            .replacing(&[&v1])
            .unwrap();
        let v3 = NodeVersion::folder(stamp(a, 3), node, None, "Three")
            .replacing(&[&v2])
            .unwrap();

        assert_eq!(head_names(&[v3.clone(), v1.clone()]), ["Three"]);
        assert!(v3.dominates(&v2) && v3.dominates(&v1));

        // An edit made offline from v1 is concurrent, not a replay.
        let offline = NodeVersion::folder(stamp(c, 4), node, None, "Offline")
            .replacing(&[&v1])
            .unwrap();
        assert_eq!(head_names(&[v3, offline]), ["Offline", "Three"]);
    }

    #[test]
    fn a_reused_device_id_keeps_both_versions() {
        let a = DeviceId::random();
        let node = NodeId::random();
        let one = NodeVersion::folder(stamp(a, 5), node, None, "One");
        let two = NodeVersion::folder(stamp(a, 5), node, None, "Two");
        assert!(!one.dominates(&two) && !two.dominates(&one));
        assert_eq!(head_names(&[one, two]), ["One", "Two"]);
    }

    #[test]
    fn forged_vectors_that_dominate_in_a_loop_still_give_heads() {
        let (a, b, c) = (DeviceId::random(), DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let forged = |device, other, name| {
            let mut v = NodeVersion::folder(stamp(device, 1), node, None, name);
            v.vv.insert(other, 1);
            v
        };
        let versions = [forged(a, b, "A"), forged(b, c, "B"), forged(c, a, "C")];
        assert_eq!(head_names(&versions), ["A", "B", "C"]);
    }

    #[test]
    fn the_most_recent_live_head_wins() {
        let (a, b, c) = (DeviceId::random(), DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let base = file(a, 1, node, "base.txt", 1_000);
        let edit = file(b, 2, node, "edit.txt", 2_000)
            .replacing(&[&base])
            .unwrap();
        let mut deletion = NodeVersion::tombstone(stamp(c, 3), &[&base]).unwrap();
        deletion.time = 3_000;

        let versions = [base, edit, deletion];
        let history = History::new(ids(&versions));
        let heads = history.heads().unwrap();
        assert_eq!(heads.len(), 2);
        let (_, winner) = heads.winner();
        assert_eq!(
            winner.name, "edit.txt",
            "an edit wins over a later deletion"
        );
        assert!(heads.in_conflict());
    }

    #[test]
    fn ties_go_to_the_lowest_version_id() {
        let (a, b) = (DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let versions = [file(a, 1, node, "a.txt", 5), file(b, 1, node, "b.txt", 5)];
        let tagged = ids(&versions);
        let lowest = tagged.iter().min_by_key(|(id, _)| *id).unwrap().0;
        let reversed: Vec<_> = tagged.iter().rev().copied().collect();
        for order in [tagged.clone(), reversed] {
            let heads = History::new(order).heads().unwrap();
            assert_eq!(heads.winner().0, lowest);
        }
    }

    #[test]
    fn every_head_a_tombstone_gives_the_latest_tombstone() {
        let (a, b, c) = (DeviceId::random(), DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let live = NodeVersion::folder(stamp(a, 1), node, None, "Old");
        let mut first = NodeVersion::tombstone(stamp(b, 2), &[&live]).unwrap();
        first.time = 10;
        let mut second = NodeVersion::tombstone(stamp(c, 3), &[&live]).unwrap();
        second.time = 20;
        second.name = "Elsewhere".into();
        let versions = [live, first, second];
        let heads = History::new(ids(&versions)).heads().unwrap();
        let (_, winner) = heads.winner();
        assert_eq!(winner.node_type, NodeType::Deleted);
        assert_eq!((winner.time, winner.name.as_str()), (20, "Elsewhere"));
    }

    #[test]
    fn heads_that_only_differ_in_who_wrote_them_are_no_conflict() {
        let (a, b) = (DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let first = NodeVersion::folder(stamp(a, 1), node, None, "Same");
        let second = first.rewritten(stamp(b, 7), &[]).unwrap();
        let versions = [first, second];
        let heads = History::new(ids(&versions)).heads().unwrap();
        assert_eq!(heads.len(), 2);
        assert!(!heads.in_conflict());
    }

    #[test]
    fn differing_files_become_identical_copies_on_every_device() {
        let (a, b) = (DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let mut mine = file(a, 1, node, "notes.txt", 1_763_856_000_000);
        mine.parent = Some(NodeId::random());
        let theirs = file(b, 1, node, "notes.txt", 1_763_856_000_001);
        let versions = [mine.clone(), theirs];
        let heads = History::new(ids(&versions)).heads().unwrap();
        assert!(heads.in_conflict());

        let copies = |device| {
            let mut clock = DeviceClock::new(device);
            heads.conflict_copies(&mut clock, 0).unwrap()
        };
        let (on_a, on_b) = (copies(a), copies(b));
        assert_eq!(on_a.len(), 1);
        let copy = &on_a[0];
        assert_eq!(copy.name, "notes (conflict 2025-11-23).txt");
        assert_eq!(copy.file, mine.file, "the losing head's content");
        assert_eq!(copy.parent, heads.winner().1.parent);
        assert_ne!(copy.node, node);
        assert_eq!(copy.vv.len(), 1);
        assert!(copy.same_state(&on_b[0]));
    }

    #[test]
    fn renames_alone_make_no_copies() {
        let (a, b) = (DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let base = file(a, 1, node, "a.txt", 1);
        let mut renamed = base.rewritten(stamp(b, 2), &[]).unwrap();
        renamed.name = "b.txt".into();
        let versions = [base, renamed];
        let heads = History::new(ids(&versions)).heads().unwrap();
        assert!(heads.in_conflict());
        let mut clock = DeviceClock::new(a);
        assert!(heads.conflict_copies(&mut clock, 0).unwrap().is_empty());
    }
}
