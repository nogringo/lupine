//! An open drive on this device: its keys, what the device knows of the
//! drive, and its relays. Syncs, writes versions, settles conflicts and the
//! tree's repairs, and compacts.

use std::collections::{BTreeSet, HashMap, HashSet};

use futures::future::join_all;
use futures::{Stream, StreamExt, stream};
use nostr::event::{Event, EventId, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::types::RelayUrl;
use unicode_normalization::UnicodeNormalization;

use crate::content::{ContentDescriptor, FileKey};
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::ids::{NodeId, VersionId};
use crate::node::{DeviceClock, FileMeta, NodeType, NodeVersion, unix_millis};
use crate::relays::{Comparison, Relays, Sent};
use crate::replica::{Holdings, Replica, Retention};
use crate::tree::Tree;

#[cfg(test)]
mod tests;

const SEND_CONCURRENCY: usize = 16;
/// Conflict copies and repairs can collide again, and need another pass.
const SETTLE_PASSES: usize = 3;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub changed: BTreeSet<NodeId>,
    /// Events downloaded.
    pub received: usize,
    /// Events sent to relays that lacked them.
    pub repaired: usize,
    /// Relays that could not be synced. Their repairs wait for a sync that
    /// reaches them.
    pub failed: Vec<(RelayUrl, Error)>,
}

#[derive(Debug)]
pub struct Published {
    pub version: VersionId,
    pub wrap: EventId,
    /// A wrap no relay accepted stays on this device, and the next sync
    /// sends it.
    pub sent: Sent,
}

pub struct DriveSession {
    keys: DriveKeys,
    epoch: u32,
    clock: DeviceClock,
    replica: Replica,
    relays: Relays,
    retention: Retention,
    complete: bool,
}

impl DriveSession {
    pub async fn open(
        keys: DriveKeys,
        epoch: u32,
        clock: DeviceClock,
        relays: impl IntoIterator<Item = RelayUrl>,
    ) -> Result<Self> {
        let relays = Relays::for_drive(&keys, relays).await?;
        Ok(Self {
            replica: Replica::new(keys.public_key()),
            keys,
            epoch,
            clock,
            relays,
            retention: Retention::default(),
            complete: false,
        })
    }

    pub fn with_retention(mut self, retention: Retention) -> Self {
        self.retention = retention;
        self
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    /// To persist, so the device never reuses a sequence number.
    pub fn clock(&self) -> DeviceClock {
        self.clock
    }

    pub fn replica(&self) -> &Replica {
        &self.replica
    }

    pub fn relays(&self) -> &Relays {
        &self.relays
    }

    /// Every drive relay has been synced at least once.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn tree(&self) -> Tree {
        self.replica.tree(self.complete)
    }

    fn wraps(&self) -> Filter {
        Filter::new().kind(Kind::GiftWrap).pubkey(self.public_key())
    }

    fn deletions(&self) -> Filter {
        Filter::new()
            .kind(Kind::EventDeletion)
            .author(self.public_key())
    }

    /// Reconciles with every drive relay: downloads what this device lacks,
    /// deletions first, then sends each relay the wraps it lacks and the
    /// deletions of wraps it still holds.
    pub async fn sync(&mut self) -> Result<SyncReport> {
        let (wraps, deletions) = (self.wraps(), self.deletions());
        let wrap_items = self.replica.wrap_items();
        let deletion_items = self.replica.deletion_items();
        let urls = self.relays.urls().to_vec();
        let relays = &self.relays;
        let comparisons = join_all(urls.iter().map(|url| async {
            let deletions = relays.compare(url, &deletions, &deletion_items).await?;
            let wraps = relays.compare(url, &wraps, &wrap_items).await?;
            Ok::<_, Error>((deletions, wraps))
        }))
        .await;

        let mut report = SyncReport::default();
        let mut synced = Vec::new();
        for (url, comparison) in urls.into_iter().zip(comparisons) {
            match comparison {
                Ok((deletions, wraps)) => synced.push((url, deletions, wraps)),
                Err(e) => report.failed.push((url, e)),
            }
        }

        let by_deletions = synced.iter_mut().map(|(url, d, _)| (&*url, d)).collect();
        self.download(&deletions, by_deletions, &mut report).await;
        let by_wraps = synced.iter_mut().map(|(url, _, w)| (&*url, w)).collect();
        self.download(&wraps, by_wraps, &mut report).await;

        let mut compared: HashSet<EventId> = wrap_items.iter().map(|(id, _)| *id).collect();
        compared.extend(synced.iter().flat_map(|(_, _, w)| w.held.iter().copied()));
        let holdings = Holdings {
            compared,
            relays: synced
                .into_iter()
                .map(|(url, _, wraps)| (url, wraps.held))
                .collect(),
        };
        report.repaired = self.send_repairs(&holdings).await;

        if report.failed.is_empty() {
            self.complete = true;
        }
        Ok(report)
    }

    async fn download(
        &mut self,
        filter: &Filter,
        comparisons: Vec<(&RelayUrl, &mut Comparison)>,
        report: &mut SyncReport,
    ) {
        let mut wanted: HashSet<EventId> = comparisons
            .iter()
            .flat_map(|(_, c)| c.missing.iter().copied())
            .collect();
        for (url, comparison) in comparisons {
            for event in std::mem::take(&mut comparison.events) {
                self.ingest(&event, report);
            }
            let ids: Vec<EventId> = comparison
                .missing
                .iter()
                .filter(|id| wanted.contains(id))
                .copied()
                .collect();
            if ids.is_empty() {
                continue;
            }
            match self.relays.fetch(url, filter, &ids).await {
                Ok(events) => {
                    for event in events {
                        if wanted.remove(&event.id) {
                            self.ingest(&event, report);
                        }
                    }
                }
                Err(e) => report.failed.push((url.clone(), e)),
            }
        }
    }

    fn ingest(&mut self, event: &Event, report: &mut SyncReport) {
        report.received += 1;
        report
            .changed
            .extend(self.replica.insert(&self.keys, event));
    }

    async fn send_repairs(&self, holdings: &Holdings) -> usize {
        let mut by_event: HashMap<EventId, (Event, Vec<RelayUrl>)> = HashMap::new();
        for (relay, event) in self.replica.repairs(holdings) {
            by_event
                .entry(event.id)
                .or_insert_with(|| (event, Vec::new()))
                .1
                .push(relay);
        }
        stream::iter(by_event.into_values())
            .map(|(event, to)| async move { self.relays.send(&event, &to).await.accepted.len() })
            .buffer_unordered(SEND_CONCURRENCY)
            .fold(0, |total, accepted| async move { total + accepted })
            .await
    }

    /// New wraps and deletions as relays receive them. Pass each to
    /// [`receive`](Self::receive). Events missed while disconnected come
    /// with the next [`sync`](Self::sync).
    pub async fn watch(&self) -> Result<impl Stream<Item = Event> + Send + use<>> {
        self.relays
            .subscribe(vec![self.wraps(), self.deletions()])
            .await
    }

    /// Returns the nodes whose versions changed.
    pub fn receive(&mut self, event: &Event) -> BTreeSet<NodeId> {
        self.replica.insert(&self.keys, event)
    }

    /// Wraps `version` and sends it to every drive relay. Run
    /// `BlossomStore::replicate` on each blob it references first: another
    /// device may have collected a blob this version reuses.
    pub async fn publish(&mut self, version: NodeVersion) -> Result<Published> {
        let (id, event) = self.keys.wrap(&version)?;
        self.replica.insert(&self.keys, &event);
        let sent = self.relays.send_all(&event).await;
        Ok(Published {
            version: id,
            wrap: event.id,
            sent,
        })
    }

    pub async fn create_folder(
        &mut self,
        parent: Option<NodeId>,
        name: &str,
    ) -> Result<(NodeId, Published)> {
        self.check_parent(parent)?;
        let node = NodeId::random();
        let stamp = self.clock.stamp(self.epoch);
        let published = self
            .publish(NodeVersion::folder(stamp, node, parent, name))
            .await?;
        Ok((node, published))
    }

    pub async fn create_file(
        &mut self,
        parent: Option<NodeId>,
        name: &str,
        key: &FileKey,
        content: ContentDescriptor,
    ) -> Result<(NodeId, Published)> {
        self.check_parent(parent)?;
        let node = NodeId::random();
        let stamp = self.clock.stamp(self.epoch);
        let version = NodeVersion::file(stamp, node, parent, name, key, content);
        Ok((node, self.publish(version).await?))
    }

    /// Renames or moves `node`, replacing every head it has.
    pub async fn move_to(
        &mut self,
        node: NodeId,
        parent: Option<NodeId>,
        name: &str,
    ) -> Result<Published> {
        self.check_parent(parent)?;
        let next = self.next_version(&node, |next| {
            next.parent = parent;
            next.name = name.nfc().collect();
            Ok(())
        })?;
        self.publish(next).await
    }

    /// New content for the file `node`, replacing every head it has.
    pub async fn replace_content(
        &mut self,
        node: NodeId,
        key: &FileKey,
        content: ContentDescriptor,
        modified: Option<i64>,
    ) -> Result<Published> {
        let next = self.next_version(&node, |next| {
            if next.node_type != NodeType::File {
                return Err(Error::Malformed("only files have content"));
            }
            next.file = Some(FileMeta {
                key: key.root().clone(),
                content,
            });
            next.modified = modified;
            Ok(())
        })?;
        self.publish(next).await
    }

    fn next_version(
        &mut self,
        node: &NodeId,
        change: impl FnOnce(&mut NodeVersion) -> Result<()>,
    ) -> Result<NodeVersion> {
        let heads = self
            .replica
            .heads(node)
            .ok_or(Error::Malformed("unknown node"))?;
        let mut next = heads.winner().1.clone();
        if next.is_tombstone() {
            // Only a folder that something inside revives shows when deleted.
            next.node_type = NodeType::Folder;
        }
        change(&mut next)?;
        next.rewritten(self.clock.stamp(self.epoch), &heads.versions())
    }

    /// Tombstones for `node` and everything under it, deepest first, so a
    /// deletion cut short never revives a folder. Stops at the first
    /// tombstone no relay accepts.
    pub async fn delete(&mut self, node: NodeId) -> Result<usize> {
        let tree = self.tree();
        if tree.get(&node).is_none() {
            return Err(Error::Malformed("unknown node"));
        }
        let mut order = Vec::new();
        let mut stack = vec![(node, false)];
        while let Some((current, expanded)) = stack.pop() {
            if expanded {
                order.push(current);
                continue;
            }
            stack.push((current, true));
            stack.extend(tree.children(Some(current)).map(|e| (e.node, false)));
        }
        for current in &order {
            let heads = self
                .replica
                .heads(current)
                .ok_or(Error::Malformed("unknown node"))?;
            let (winner_id, winner) = heads.winner();
            let mut replaced = vec![winner];
            replaced.extend(
                heads
                    .iter()
                    .filter(|(id, _)| *id != winner_id)
                    .map(|(_, v)| v),
            );
            let tombstone = NodeVersion::tombstone(self.clock.stamp(self.epoch), &replaced)?;
            let published = self.publish(tombstone).await?;
            if published.sent.accepted.is_empty() {
                return Err(Error::NotPublished(published.sent.reasons()));
            }
        }
        Ok(order.len())
    }

    fn check_parent(&self, parent: Option<NodeId>) -> Result<()> {
        let Some(parent) = parent else {
            return Ok(());
        };
        let heads = self
            .replica
            .heads(&parent)
            .ok_or(Error::Malformed("unknown parent"))?;
        match heads.winner().1.node_type {
            NodeType::File => Err(Error::Malformed("the parent is a file")),
            NodeType::Folder | NodeType::Deleted => Ok(()),
        }
    }

    /// Resolves conflicts and publishes the states the tree rules show but
    /// no version holds. Waits for a full sync: until then, the versions
    /// that settle things may only be late.
    pub async fn settle(&mut self) -> Result<usize> {
        if !self.complete {
            return Ok(0);
        }
        let mut published = 0;
        for _ in 0..SETTLE_PASSES {
            let versions = self.settlement()?;
            if versions.is_empty() {
                break;
            }
            for version in versions {
                self.publish(version).await?;
                published += 1;
            }
        }
        Ok(published)
    }

    fn settlement(&mut self) -> Result<Vec<NodeVersion>> {
        let tree = self.replica.tree(true);
        let nodes: BTreeSet<NodeId> = self.replica.node_ids().collect();
        let mut versions = Vec::new();
        for node in nodes {
            let Some(heads) = self.replica.heads(&node) else {
                continue;
            };
            let repair = tree.get(&node).filter(|entry| entry.needs_repair());
            let conflict = heads.in_conflict();
            if repair.is_none() && !conflict {
                continue;
            }
            let mut next = heads.winner().1.clone();
            if let Some(entry) = repair {
                if entry.revived {
                    next.node_type = NodeType::Folder;
                }
                if entry.cycle_broken {
                    next.parent = None;
                }
                if entry.renamed && !entry.orphan {
                    next.name.clone_from(&entry.name);
                }
            }
            let stamp = self.clock.stamp(self.epoch);
            versions.push(next.rewritten(stamp, &heads.versions())?);
            if conflict {
                for copy in heads.conflict_copies(&mut self.clock, self.epoch)? {
                    if self.replica.heads(&copy.node).is_none() {
                        versions.push(copy);
                    }
                }
            }
        }
        Ok(versions)
    }

    /// Deletes, as the wraps' recipient, the superseded versions past the
    /// history window, the tombstones past their lifetime and the junk.
    /// Waits for a full sync. Returns the number of wraps deleted.
    pub async fn compact(&mut self) -> Result<usize> {
        if !self.complete {
            return Ok(0);
        }
        let doomed = self.replica.compaction(unix_millis(), &self.retention);
        for deletion in self.keys.deletions(doomed.iter().copied())? {
            // Kept even if no relay takes it: the next sync sends it.
            self.replica.insert(&self.keys, &deletion);
            self.relays.send_all(&deletion).await;
        }
        Ok(doomed.len())
    }

    /// Republishes `event`, unchanged, to the drive relays that lack it:
    /// the descriptor this device read, or the drive's lists.
    pub async fn keep(&self, event: &Event) -> Sent {
        let lacking = self.relays.lacking(event).await;
        self.relays.send(event, &lacking).await
    }

    pub async fn shutdown(&self) {
        self.relays.shutdown().await;
    }
}
