//! What this device knows of the drive's events: every gift wrap addressed
//! to the drive and every drive deletion, the versions the wraps carry, and
//! from those the tree, compaction and the repairs relays need. Indexes are
//! kept in memory; every change is journaled for the local database, which
//! also keeps the events relays may need.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use nostr::event::{Event, EventId, Kind};
use nostr::key::PublicKey;
use nostr::types::{RelayUrl, Timestamp};

use crate::drive::{DriveKeys, Unwrapped, deleted_wraps};
use crate::graph::{Heads, History};
use crate::ids::{NodeId, VersionId};
use crate::node::NodeVersion;
use crate::tree::Tree;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long superseded versions and tombstones stay on the relays.
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    /// The history window: a superseded version goes once a version that
    /// dominates it is older than this. A client setting.
    pub history: Duration,
    /// The tombstone lifetime: a node whose heads are all tombstones goes
    /// once the latest is older than this.
    pub tombstones: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            history: 30 * DAY,
            tombstones: 365 * DAY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapState {
    Version(VersionId),
    /// Decrypts with the drive keys but holds no version this client reads.
    Unreadable,
    Junk,
    /// Named by a drive deletion.
    Deleted,
}

impl WrapState {
    /// Relays that lack the wrap should get it.
    pub fn is_kept(&self) -> bool {
        matches!(self, Self::Version(_) | Self::Unreadable)
    }
}

struct Wrap {
    created_at: Timestamp,
    state: WrapState,
}

/// A change to what the replica holds, for the local database.
#[derive(Clone, Debug)]
pub enum Record {
    /// A wrap seen for the first time, or a known one now deleted. `event`
    /// comes with the wraps that relays lacking them should get.
    Wrap {
        id: EventId,
        created_at: Timestamp,
        state: WrapState,
        event: Option<Event>,
    },
    Version {
        id: VersionId,
        version: NodeVersion,
    },
    /// Its last wrap was deleted.
    VersionGone(VersionId),
    Deletion(Event),
    Deleted {
        wrap: EventId,
        by: EventId,
    },
}

impl Record {
    pub fn changes_versions(&self) -> bool {
        matches!(self, Self::Version { .. } | Self::VersionGone(_))
    }
}

/// What the local database holds, to rebuild a replica from.
#[derive(Default)]
pub struct Snapshot {
    pub wraps: Vec<(EventId, Timestamp, WrapState)>,
    pub versions: Vec<(VersionId, NodeVersion)>,
    pub deletions: Vec<(EventId, Timestamp)>,
    /// Each deleted wrap, and the first deletion seen naming it.
    pub deleted: Vec<(EventId, EventId)>,
}

/// What a full sync learned of the wraps it compared: which relays hold
/// which.
#[derive(Debug, Default)]
pub struct Holdings {
    pub compared: HashSet<EventId>,
    pub relays: HashMap<RelayUrl, HashSet<EventId>>,
}

pub struct Replica {
    drive: PublicKey,
    wraps: HashMap<EventId, Wrap>,
    versions: HashMap<VersionId, NodeVersion>,
    /// Wraps carrying each version, deleted ones excluded.
    carriers: HashMap<VersionId, BTreeSet<EventId>>,
    nodes: HashMap<NodeId, BTreeSet<VersionId>>,
    deletions: HashMap<EventId, Timestamp>,
    /// Each deleted wrap, and the first deletion seen naming it.
    deleted_by: HashMap<EventId, EventId>,
    journal: Vec<Record>,
}

impl Replica {
    pub fn new(drive: PublicKey) -> Self {
        Self {
            drive,
            wraps: HashMap::new(),
            versions: HashMap::new(),
            carriers: HashMap::new(),
            nodes: HashMap::new(),
            deletions: HashMap::new(),
            deleted_by: HashMap::new(),
            journal: Vec::new(),
        }
    }

    /// Without decrypting or verifying anything again.
    pub fn restore(drive: PublicKey, snapshot: Snapshot) -> Self {
        let mut replica = Self::new(drive);
        replica.versions = snapshot.versions.into_iter().collect();
        for (id, created_at, state) in snapshot.wraps {
            if let WrapState::Version(version) = state
                && let Some(node) = replica.versions.get(&version).map(|v| v.node)
            {
                replica.carriers.entry(version).or_default().insert(id);
                replica.nodes.entry(node).or_default().insert(version);
            }
            replica.wraps.insert(id, Wrap { created_at, state });
        }
        replica
            .versions
            .retain(|id, _| replica.carriers.contains_key(id));
        replica.deletions = snapshot.deletions.into_iter().collect();
        replica.deleted_by = snapshot.deleted.into_iter().collect();
        replica
    }

    /// The changes since the last call, oldest first.
    pub fn take_journal(&mut self) -> Vec<Record> {
        std::mem::take(&mut self.journal)
    }

    /// Takes a wrap or a drive deletion, from a relay or from this device.
    /// Returns the nodes whose versions changed.
    pub fn insert(&mut self, keys: &DriveKeys, event: &Event) -> BTreeSet<NodeId> {
        match event.kind {
            Kind::EventDeletion => self.insert_deletion(event),
            Kind::GiftWrap => self.insert_wrap(keys, event).into_iter().collect(),
            _ => BTreeSet::new(),
        }
    }

    fn insert_deletion(&mut self, event: &Event) -> BTreeSet<NodeId> {
        let named = deleted_wraps(&self.drive, event);
        if named.is_empty() || self.deletions.contains_key(&event.id) {
            return BTreeSet::new();
        }
        self.deletions.insert(event.id, event.created_at);
        self.journal.push(Record::Deletion(event.clone()));
        let mut changed = BTreeSet::new();
        for wrap in named {
            if let Entry::Vacant(slot) = self.deleted_by.entry(wrap) {
                slot.insert(event.id);
                self.journal.push(Record::Deleted { wrap, by: event.id });
            }
            changed.extend(self.drop_wrap(&wrap));
        }
        changed
    }

    fn insert_wrap(&mut self, keys: &DriveKeys, event: &Event) -> Option<NodeId> {
        if self.wraps.contains_key(&event.id) {
            return None;
        }
        let unwrapped = keys.unwrap(event)?;
        let mut changed = None;
        let state = if self.deleted_by.contains_key(&event.id) {
            WrapState::Deleted
        } else {
            match unwrapped {
                Unwrapped::Version(id, version) => {
                    let node = version.node;
                    self.carriers.entry(id).or_default().insert(event.id);
                    if self.nodes.entry(node).or_default().insert(id) {
                        changed = Some(node);
                        self.journal.push(Record::Version {
                            id,
                            version: (*version).clone(),
                        });
                        self.versions.insert(id, *version);
                    }
                    WrapState::Version(id)
                }
                Unwrapped::Unreadable(_) => WrapState::Unreadable,
                Unwrapped::Junk => WrapState::Junk,
            }
        };
        self.journal.push(Record::Wrap {
            id: event.id,
            created_at: event.created_at,
            state,
            event: state.is_kept().then(|| event.clone()),
        });
        let wrap = Wrap {
            created_at: event.created_at,
            state,
        };
        self.wraps.insert(event.id, wrap);
        changed
    }

    /// The node that lost a version, when the wrap carried its last copy.
    fn drop_wrap(&mut self, wrap: &EventId) -> Option<NodeId> {
        let record = self.wraps.get_mut(wrap)?;
        let previous = std::mem::replace(&mut record.state, WrapState::Deleted);
        if previous == WrapState::Deleted {
            return None;
        }
        self.journal.push(Record::Wrap {
            id: *wrap,
            created_at: record.created_at,
            state: WrapState::Deleted,
            event: None,
        });
        let WrapState::Version(id) = previous else {
            return None;
        };
        let carriers = self.carriers.get_mut(&id)?;
        carriers.remove(wrap);
        if !carriers.is_empty() {
            return None;
        }
        self.carriers.remove(&id);
        let version = self.versions.remove(&id)?;
        self.journal.push(Record::VersionGone(id));
        let node = version.node;
        if let Some(versions) = self.nodes.get_mut(&node) {
            versions.remove(&id);
            if versions.is_empty() {
                self.nodes.remove(&node);
            }
        }
        Some(node)
    }

    pub fn drive(&self) -> PublicKey {
        self.drive
    }

    pub fn version(&self, id: &VersionId) -> Option<&NodeVersion> {
        self.versions.get(id)
    }

    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    pub fn history(&self, node: &NodeId) -> History<'_> {
        let ids = self.nodes.get(node).into_iter().flatten();
        History::new(ids.map(|id| (*id, &self.versions[id])))
    }

    pub fn heads(&self, node: &NodeId) -> Option<Heads<'_>> {
        self.history(node).heads()
    }

    /// `complete` once every drive relay has been synced.
    pub fn tree(&self, complete: bool) -> Tree {
        let winners: Vec<_> = self
            .nodes
            .keys()
            .filter_map(|node| self.heads(node).map(|heads| heads.winner()))
            .collect();
        Tree::build(winners, complete)
    }

    /// Negentropy items for every wrap seen, deleted and junk ones too, so
    /// a sync tells which relays still hold them.
    pub fn wrap_items(&self) -> Vec<(EventId, Timestamp)> {
        self.wraps
            .iter()
            .map(|(id, wrap)| (*id, wrap.created_at))
            .collect()
    }

    pub fn deletion_items(&self) -> Vec<(EventId, Timestamp)> {
        self.deletions
            .iter()
            .map(|(id, created_at)| (*id, *created_at))
            .collect()
    }

    pub fn is_deleted(&self, wrap: &EventId) -> bool {
        self.deleted_by.contains_key(wrap)
    }

    /// Wraps to delete at `now` (Unix ms): those carrying a superseded
    /// version past the history window, every wrap of a node whose
    /// tombstones outlived their lifetime, and wraps that fail to decrypt.
    pub fn compaction(&self, now: i64, retention: &Retention) -> Vec<EventId> {
        let cutoff = |window: Duration| now.saturating_sub(window.as_millis() as i64);
        let (history_cutoff, tombstone_cutoff) =
            (cutoff(retention.history), cutoff(retention.tombstones));

        let mut doomed = BTreeSet::new();
        for node in self.nodes.keys() {
            let history = self.history(node);
            let Some(heads) = history.heads() else {
                continue;
            };
            let expired = heads.iter().all(|(_, head)| head.is_tombstone())
                && heads.iter().all(|(_, head)| head.time < tombstone_cutoff);
            for (id, version) in history.versions() {
                let superseded = || {
                    history
                        .versions()
                        .iter()
                        .any(|(_, w)| w.dominates(version) && w.time < history_cutoff)
                };
                if expired || superseded() {
                    doomed.insert(*id);
                }
            }
        }

        let mut wraps: Vec<EventId> = doomed
            .iter()
            .flat_map(|id| self.carriers.get(id).into_iter().flatten())
            .copied()
            .chain(
                self.wraps
                    .iter()
                    .filter(|(_, wrap)| wrap.state == WrapState::Junk)
                    .map(|(id, _)| *id),
            )
            .collect();
        wraps.sort();
        wraps
    }

    /// What each relay needs so relays converge: the wraps it lacks, and
    /// the deletions naming wraps it still holds. Junk is not spread. The
    /// local database holds the events.
    pub fn repairs(&self, holdings: &Holdings) -> Vec<(RelayUrl, EventId)> {
        let mut repairs = Vec::new();
        for (relay, held) in &holdings.relays {
            let mut deletions = BTreeSet::new();
            for wrap in &holdings.compared {
                let Some(record) = self.wraps.get(wrap) else {
                    continue;
                };
                match record.state {
                    state if state.is_kept() && !held.contains(wrap) => {
                        repairs.push((relay.clone(), *wrap));
                    }
                    WrapState::Deleted if held.contains(wrap) => {
                        deletions.extend(self.deleted_by.get(wrap));
                    }
                    _ => {}
                }
            }
            repairs.extend(deletions.into_iter().map(|id| (relay.clone(), id)));
        }
        repairs
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;
    use crate::ids::DeviceId;
    use crate::node::{DeviceClock, Stamp};

    const NOW: i64 = 1_800_000_000_000;

    struct Setup {
        keys: DriveKeys,
        replica: Replica,
        clock: DeviceClock,
    }

    impl Setup {
        fn new() -> Self {
            let keys = DriveKeys::generate();
            Self {
                replica: Replica::new(keys.public_key()),
                keys,
                clock: DeviceClock::new(DeviceId::random()),
            }
        }

        fn stamp(&mut self) -> Stamp {
            self.clock.stamp(0)
        }

        fn publish(&mut self, version: &NodeVersion) -> Event {
            let (_, event) = self.keys.wrap(version).unwrap();
            self.replica.insert(&self.keys, &event);
            event
        }

        fn folder(&mut self, name: &str, time: i64) -> NodeVersion {
            let stamp = self.stamp();
            let mut version = NodeVersion::folder(stamp, NodeId::random(), None, name);
            version.time = time;
            version
        }

        fn edit(&mut self, base: &NodeVersion, name: &str, time: i64) -> NodeVersion {
            let stamp = self.stamp();
            let mut version = base.rewritten(stamp, &[base]).unwrap();
            version.name = name.into();
            version.time = time;
            version
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<_> = self
                .replica
                .tree(true)
                .entries()
                .map(|e| e.name.clone())
                .collect();
            names.sort();
            names
        }
    }

    #[test]
    fn versions_count_once_whatever_carries_them() {
        let mut s = Setup::new();
        let version = s.folder("Photos", NOW);
        let first = s.publish(&version);
        let again = s.publish(&version);
        assert_ne!(first.id, again.id);
        assert!(s.replica.insert(&s.keys, &first).is_empty());
        assert_eq!(s.replica.history(&version.node).versions().len(), 1);
        assert_eq!(s.replica.wrap_items().len(), 2);
    }

    #[test]
    fn deletions_remove_a_version_with_its_last_wrap() {
        let mut s = Setup::new();
        let version = s.folder("Photos", NOW);
        let first = s.publish(&version);
        let second = s.publish(&version);

        let deletion = &s.keys.deletions([first.id]).unwrap()[0];
        assert!(s.replica.insert(&s.keys, deletion).is_empty());
        assert_eq!(s.names(), ["Photos"]);

        let deletion = &s.keys.deletions([second.id]).unwrap()[0];
        assert_eq!(
            s.replica.insert(&s.keys, deletion),
            BTreeSet::from([version.node])
        );
        assert!(s.names().is_empty());
        assert_eq!(s.replica.wrap_items().len(), 2, "deleted wraps stay known");
    }

    #[test]
    fn a_wrap_deleted_before_it_arrives_is_ignored() {
        let mut s = Setup::new();
        let version = s.folder("Photos", NOW);
        let (_, event) = s.keys.wrap(&version).unwrap();
        let deletion = &s.keys.deletions([event.id]).unwrap()[0];
        s.replica.insert(&s.keys, deletion);
        assert!(s.replica.insert(&s.keys, &event).is_empty());
        assert!(s.names().is_empty());
        assert!(s.replica.is_deleted(&event.id));
    }

    #[test]
    fn deletions_from_anyone_but_the_drive_are_ignored() {
        let mut s = Setup::new();
        let version = s.folder("Photos", NOW);
        let wrap = s.publish(&version);
        let forged = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::event(wrap.id))
            .finalize(&Keys::generate())
            .unwrap();
        s.replica.insert(&s.keys, &forged);
        assert_eq!(s.names(), ["Photos"]);
        assert!(s.replica.deletion_items().is_empty());
    }

    #[test]
    fn compaction_keeps_history_for_the_window() {
        let mut s = Setup::new();
        let retention = Retention::default();
        let window = retention.history.as_millis() as i64;
        let old = s.folder("Old", NOW - 2 * window);
        let middle = s.edit(&old, "Middle", NOW - window - 1);
        let recent = s.edit(&middle, "Recent", NOW - 1);
        let old_wrap = s.publish(&old);
        s.publish(&middle);
        s.publish(&recent);

        assert_eq!(
            s.replica.compaction(NOW, &retention),
            [old_wrap.id],
            "middle is superseded by a version inside the window"
        );
    }

    #[test]
    fn compaction_drops_expired_tombstones_and_junk() {
        let mut s = Setup::new();
        let retention = Retention::default();
        let lifetime = retention.tombstones.as_millis() as i64;
        let live = s.folder("Gone", NOW - 2 * lifetime);
        let stamp = s.stamp();
        let mut tombstone = NodeVersion::tombstone(stamp, &[&live]).unwrap();
        tombstone.time = NOW - lifetime - 1;
        let live_wrap = s.publish(&live);
        let tombstone_wrap = s.publish(&tombstone);

        let recent = s.folder("Recent", NOW);
        let stamp = s.stamp();
        let mut fresh = NodeVersion::tombstone(stamp, &[&recent]).unwrap();
        fresh.time = NOW - retention.history.as_millis() as i64 - 1;
        s.publish(&recent);
        s.publish(&fresh);

        let junk = EventBuilder::new(Kind::GiftWrap, "spam")
            .tag(Tag::public_key(s.keys.public_key()))
            .finalize(&Keys::generate())
            .unwrap();
        s.replica.insert(&s.keys, &junk);

        let mut expected = [live_wrap.id, tombstone_wrap.id, junk.id];
        let compaction = s.replica.compaction(NOW, &retention);
        expected.sort();
        let recent_wraps: Vec<_> = compaction
            .iter()
            .filter(|id| !expected.contains(id))
            .collect();
        assert_eq!(
            recent_wraps.len(),
            1,
            "the live version the fresh tombstone replaced"
        );
        assert!(expected.iter().all(|id| compaction.contains(id)));
    }

    #[test]
    fn repairs_spread_wraps_and_deletions() {
        let mut s = Setup::new();
        let kept = s.folder("Kept", NOW);
        let kept_wrap = s.publish(&kept);
        let gone = s.folder("Gone", NOW);
        let gone_wrap = s.publish(&gone);
        let deletion = s.keys.deletions([gone_wrap.id]).unwrap().remove(0);
        s.replica.insert(&s.keys, &deletion);
        let junk = EventBuilder::new(Kind::GiftWrap, "spam")
            .tag(Tag::public_key(s.keys.public_key()))
            .finalize(&Keys::generate())
            .unwrap();
        s.replica.insert(&s.keys, &junk);

        let up_to_date = RelayUrl::parse("wss://a.example.com").unwrap();
        let behind = RelayUrl::parse("wss://b.example.com").unwrap();
        let holdings = Holdings {
            compared: [kept_wrap.id, gone_wrap.id, junk.id].into(),
            relays: HashMap::from([
                (up_to_date.clone(), [kept_wrap.id].into()),
                (behind.clone(), [gone_wrap.id, junk.id].into()),
            ]),
        };
        let mut sent: Vec<_> = s
            .replica
            .repairs(&holdings)
            .into_iter()
            .map(|(relay, id)| (relay.to_string(), id))
            .collect();
        sent.sort();
        let mut expected = vec![
            (behind.to_string(), kept_wrap.id),
            (behind.to_string(), deletion.id),
        ];
        expected.sort();
        assert_eq!(sent, expected);
    }
}
