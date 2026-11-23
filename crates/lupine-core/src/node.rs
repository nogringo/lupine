//! Node versions: the JSON a drive wraps into gift-wrap-shaped events.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::content::{ContentDescriptor, FileKey};
use crate::crypto::SymmetricKey;
use crate::encoding;
use crate::error::{Error, Result};
use crate::ids::{DeviceId, NodeId};

pub const NODE_FORMAT_VERSION: u8 = 1;
const MAX_NAME_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    File,
    Folder,
    /// Tombstone, so other devices learn of the deletion.
    Deleted,
}

/// Who writes a version: the device, its next sequence number and the
/// current drive epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub device: DeviceId,
    pub seq: u64,
    pub epoch: u32,
}

impl Stamp {
    /// Never below the clock, so a device that lost its counter does not
    /// reuse a sequence number.
    pub fn next(device: DeviceId, previous_seq: u64, epoch: u32) -> Self {
        Self {
            device,
            seq: previous_seq.saturating_add(1).max(unix_millis() as u64),
            epoch,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeVersion {
    pub v: u8,
    pub node: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub epoch: u32,
    /// When this version was written, in Unix milliseconds. The event's own
    /// `created_at` is randomized, like any gift wrap's.
    pub time: i64,
    pub device: DeviceId,
    /// This version's own sequence number under `device`, and for every
    /// other device the highest one among the versions it replaces.
    pub vv: BTreeMap<DeviceId, u64>,
    /// `None` at the drive root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeId>,
    pub name: String,
    /// Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMeta {
    #[serde(with = "encoding::key")]
    pub key: SymmetricKey,
    #[serde(flatten)]
    pub content: ContentDescriptor,
}

impl FileMeta {
    pub fn file_key(&self) -> FileKey {
        FileKey::from_root(self.key.clone())
    }
}

impl NodeVersion {
    pub fn folder(stamp: Stamp, node: NodeId, parent: Option<NodeId>, name: &str) -> Self {
        Self::new(stamp, node, NodeType::Folder, parent, name, None)
    }

    pub fn file(
        stamp: Stamp,
        node: NodeId,
        parent: Option<NodeId>,
        name: &str,
        key: &FileKey,
        content: ContentDescriptor,
    ) -> Self {
        let file = FileMeta {
            key: key.root().clone(),
            content,
        };
        Self::new(stamp, node, NodeType::File, parent, name, Some(file))
    }

    /// Replaces `heads`, keeping the place of `heads[0]`, the winning head,
    /// so the node can be restored.
    pub fn tombstone(stamp: Stamp, heads: &[&NodeVersion]) -> Result<Self> {
        let place = heads.first().ok_or(Error::Malformed(
            "a tombstone replaces at least one version",
        ))?;
        let tombstone = Self::new(
            stamp,
            place.node,
            NodeType::Deleted,
            place.parent,
            &place.name,
            None,
        );
        tombstone.replacing(heads)
    }

    fn new(
        stamp: Stamp,
        node: NodeId,
        node_type: NodeType,
        parent: Option<NodeId>,
        name: &str,
        file: Option<FileMeta>,
    ) -> Self {
        Self {
            v: NODE_FORMAT_VERSION,
            node,
            node_type,
            epoch: stamp.epoch,
            time: unix_millis(),
            device: stamp.device,
            vv: BTreeMap::from([(stamp.device, stamp.seq)]),
            parent,
            name: name.nfc().collect(),
            created: None,
            modified: None,
            file,
        }
    }

    /// Merges the vectors of `heads` into this version's.
    pub fn replacing(mut self, heads: &[&NodeVersion]) -> Result<Self> {
        let seq = self.seq();
        for head in heads {
            if head.node != self.node {
                return Err(Error::Malformed(
                    "a version only replaces versions of its node",
                ));
            }
            if head.vv.get(&self.device).is_some_and(|&n| n >= seq) {
                return Err(Error::Malformed("sequence numbers must increase"));
            }
            for (&device, &n) in &head.vv {
                let entry = self.vv.entry(device).or_default();
                *entry = (*entry).max(n);
            }
        }
        Ok(self)
    }

    pub fn seq(&self) -> u64 {
        self.vv.get(&self.device).copied().unwrap_or(0)
    }

    /// This version's writer had seen `other`, and `other`'s writer had not
    /// seen this one. It holds even once the versions in between are gone.
    pub fn dominates(&self, other: &NodeVersion) -> bool {
        let saw =
            |v: &NodeVersion, w: &NodeVersion| v.vv.get(&w.device).is_some_and(|&n| n >= w.seq());
        self.node == other.node && saw(self, other) && !saw(other, self)
    }

    /// Writers normalize names, readers too, in case another writer did not.
    pub(crate) fn normalize(&mut self) {
        self.name = self.name.nfc().collect();
    }

    pub fn validate(&self) -> Result<()> {
        if self.v != NODE_FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(self.v));
        }
        if self.seq() == 0 {
            return Err(Error::Malformed("the version vector lacks the writer"));
        }
        match (self.node_type, &self.file) {
            (NodeType::File, None) => return Err(Error::Malformed("file without content")),
            (NodeType::Folder | NodeType::Deleted, Some(_)) => {
                return Err(Error::Malformed("content on a non-file node"));
            }
            _ => {}
        }
        validate_name(&self.name)
    }
}

/// Versions of a node that no other version dominates.
pub fn heads(versions: &[NodeVersion]) -> Vec<&NodeVersion> {
    versions
        .iter()
        .filter(|v| !versions.iter().any(|w| w.dominates(v)))
        .collect()
}

/// Names come from the network and end up as path components on disk.
fn validate_name(name: &str) -> Result<()> {
    let invalid = name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name == "."
        || name == ".."
        || name.contains(['/', '\0']);
    if invalid {
        return Err(Error::Malformed("invalid node name"));
    }
    Ok(())
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(device: DeviceId, seq: u64) -> Stamp {
        Stamp {
            device,
            seq,
            epoch: 0,
        }
    }

    #[test]
    fn first_version_counts_only_its_writer() {
        let device = DeviceId::random();
        let json = serde_json::to_value(NodeVersion::folder(
            stamp(device, 7),
            NodeId::random(),
            None,
            "Photos",
        ))
        .unwrap();
        assert_eq!(json["vv"], serde_json::json!({ device.to_hex(): 7 }));
        assert_eq!(json["device"], device.to_hex());
        assert!(json.get("base").is_none());
        assert!(json["time"].as_i64().unwrap() > 1_700_000_000_000);
    }

    #[test]
    fn sequence_numbers_follow_the_clock() {
        let device = DeviceId::random();
        let first = Stamp::next(device, 0, 0);
        assert!(first.seq > 1_700_000_000_000);
        assert_eq!(Stamp::next(device, u64::MAX - 1, 0).seq, u64::MAX);
    }

    #[test]
    fn rejects_unsafe_names() {
        let s = stamp(DeviceId::random(), 1);
        let node = NodeId::random();
        for name in ["", ".", "..", "a/b", "nul\0", &"x".repeat(256)] {
            assert!(
                NodeVersion::folder(s, node, None, name).validate().is_err(),
                "{name:?}"
            );
        }
        assert!(
            NodeVersion::folder(s, node, None, "..hidden")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn names_are_nfc() {
        let s = stamp(DeviceId::random(), 1);
        let nfd = "cafe\u{301}";
        let version = NodeVersion::folder(s, NodeId::random(), None, nfd);
        assert_eq!(version.name, "caf\u{e9}");

        let mut read = version.clone();
        read.name = nfd.to_owned();
        read.normalize();
        assert_eq!(read.name, version.name);
    }

    #[test]
    fn tombstones_keep_the_place_of_the_node() {
        let (a, b) = (DeviceId::random(), DeviceId::random());
        let node = NodeId::random();
        let parent = Some(NodeId::random());
        let live = NodeVersion::folder(stamp(a, 1), node, parent, "Photos");
        let tombstone = NodeVersion::tombstone(stamp(b, 5), &[&live]).unwrap();
        tombstone.validate().unwrap();
        assert_eq!(tombstone.node_type, NodeType::Deleted);
        assert_eq!(
            (tombstone.parent, tombstone.name.as_str()),
            (parent, "Photos")
        );
        assert!(tombstone.dominates(&live));
        assert!(NodeVersion::tombstone(stamp(b, 6), &[]).is_err());
    }

    #[test]
    fn replacing_requires_a_newer_sequence_number() {
        let a = DeviceId::random();
        let node = NodeId::random();
        let first = NodeVersion::folder(stamp(a, 10), node, None, "A");
        assert!(
            NodeVersion::folder(stamp(a, 10), node, None, "B")
                .replacing(&[&first])
                .is_err()
        );
        assert!(
            NodeVersion::folder(stamp(a, 11), node, None, "B")
                .replacing(&[&first])
                .is_ok()
        );
        let other = NodeVersion::folder(stamp(a, 12), NodeId::random(), None, "C");
        assert!(other.replacing(&[&first]).is_err());
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

        let after_compaction = [v3.clone(), v1.clone()];
        let current = heads(&after_compaction);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].name, "Three");
        assert!(v3.dominates(&v2) && v3.dominates(&v1));

        // An edit made offline from v1 is concurrent, not a replay.
        let offline = NodeVersion::folder(stamp(c, 4), node, None, "Offline")
            .replacing(&[&v1])
            .unwrap();
        let with_offline = [v3.clone(), offline];
        assert_eq!(heads(&with_offline).len(), 2);
    }

    #[test]
    fn a_reused_device_id_keeps_both_versions() {
        let a = DeviceId::random();
        let node = NodeId::random();
        let clone_one = NodeVersion::folder(stamp(a, 5), node, None, "One");
        let clone_two = NodeVersion::folder(stamp(a, 5), node, None, "Two");
        assert!(!clone_one.dominates(&clone_two) && !clone_two.dominates(&clone_one));
        assert_eq!(heads(&[clone_one, clone_two]).len(), 2);
    }
}
