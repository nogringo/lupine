//! An open drive on this device: its keys, what the device knows of the
//! drive, and its relays. Syncs, writes versions, settles conflicts and the
//! tree's repairs, and compacts. Every step lands in the local database,
//! with the items it shows. Takes in the versions the File Provider
//! extension writes there, and sends them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use futures::{Stream, StreamExt, stream};
use nostr::event::{Event, EventId, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::types::RelayUrl;
use tokio::sync::Notify;
use unicode_normalization::UnicodeNormalization;

use crate::content::{ContentDescriptor, FileKey};
use crate::drive::{self, DriveKeys};
use crate::error::{Error, Result};
use crate::ids::{NodeId, VersionId};
use crate::items::{self, Item};
use crate::lists::{self, Changed, Latest};
use crate::node::{DeviceClock, FileMeta, NodeType, NodeVersion, unix_millis};
use crate::relays::{Comparison, Relays, Sent};
use crate::replica::{Holdings, Replica, Retention};
use crate::store::{Outgoing, Store};
use crate::task::{self, Instant};
use crate::tree::Tree;

#[cfg(test)]
mod tests;

const SEND_CONCURRENCY: usize = 16;
/// Conflict copies and repairs can collide again, and need another pass.
const SETTLE_PASSES: usize = 3;
/// A full sync while following, for what the live subscription missed.
pub const RESYNC: Duration = Duration::from_secs(10 * 60);
/// While following, after the first full sync. The history window counts
/// in days, so compacting more often would only find junk.
pub const COMPACTION: Duration = Duration::from_secs(24 * 60 * 60);
/// Live events that arrive this close together are stored together.
const BATCH_WINDOW: Duration = Duration::from_millis(250);
const BATCH_MAX: usize = 1000;
const RETRY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct SyncReport {
    pub changed: BTreeSet<NodeId>,
    /// Events downloaded.
    pub received: usize,
    /// Events sent to relays that lacked them.
    pub repaired: usize,
    /// Copies of events that relays lacked: those sent, and those refused.
    pub lacking: usize,
    /// Wraps the extension wrote that relays took, out of the outbox.
    pub sent: usize,
    /// Relays that could not be synced. Their repairs wait for a sync that
    /// reaches them.
    pub failed: Vec<(RelayUrl, Error)>,
    /// Lists another device published, applied to the local database. The
    /// session still uses the relays it opened with.
    pub lists: Changed,
    /// A key rotation retired the drive key: nothing was downloaded, and
    /// the drive needs the user's key, or a new recovery key, again.
    pub retired: bool,
}

#[derive(Debug)]
pub struct Published {
    pub version: VersionId,
    pub wrap: EventId,
    /// A wrap no relay accepted stays on this device, and the next sync
    /// sends it.
    pub sent: Sent,
}

/// What [`DriveSession::follow`] reports.
#[derive(Debug)]
pub enum Followed<'a> {
    Syncing,
    Synced(&'a SyncReport),
    /// Wraps deleted by a compaction.
    Compacted(usize),
    /// Another device changed the drive's relays or servers: reopen what
    /// uses them.
    ListsChanged,
    /// A key rotation retired the drive key. Following stopped.
    Retired,
    /// The items shown changed: file system integrations should look.
    ItemsChanged,
}

pub struct DriveSession {
    keys: DriveKeys,
    epoch: u32,
    clock: DeviceClock,
    replica: Replica,
    relays: Relays,
    complete: bool,
    store: Arc<Store>,
    /// Every drive relay was synced once, in this session or before.
    synced: bool,
    items: HashMap<NodeId, Item>,
    items_changed: bool,
    compacted: Option<Instant>,
}

enum Ended {
    Stopped,
    Resync,
    Closed,
    Retired,
}

impl DriveSession {
    /// Opens the drive set up in `store`, as it was when last closed.
    pub async fn open(store: Arc<Store>, keys: DriveKeys) -> Result<Self> {
        let drive = store
            .drive()?
            .ok_or(Error::Malformed("no drive in the local database"))?;
        if drive.pubkey != keys.public_key() {
            return Err(Error::Malformed("this key does not open the drive"));
        }
        let (clock, synced) = store.device_state()?;
        let replica = Replica::restore(keys.public_key(), store.snapshot()?);
        let items = store.shown_items()?;
        let relays = Relays::for_drive(&keys, drive.relays).await?;
        let mut session = Self {
            replica,
            keys,
            epoch: drive.epoch,
            clock,
            relays,
            complete: false,
            store,
            synced,
            items,
            items_changed: false,
            compacted: None,
        };
        // The items follow the rules of this version of the core.
        session.persist(true)?;
        Ok(session)
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn clock(&self) -> DeviceClock {
        self.clock
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
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
        self.replica.tree(self.complete || self.synced)
    }

    /// Writes what changed since the last call, and the items it changes.
    fn persist(&mut self, rebuild: bool) -> Result<()> {
        let records = self.replica.take_journal();
        let rebuild = rebuild || records.iter().any(|r| r.changes_versions());
        let shown = rebuild.then(|| items::shown(&self.replica, &self.tree()));
        let changes = match &shown {
            Some(shown) => items::diff(&self.items, shown),
            None => items::ItemChanges::default(),
        };
        self.store
            .commit(&records, self.clock, self.synced, &changes)?;
        if let Some(shown) = shown {
            self.items = shown;
        }
        self.items_changed |= !changes.is_empty();
        Ok(())
    }

    fn wraps(&self) -> Filter {
        Filter::new().kind(Kind::GiftWrap).pubkey(self.public_key())
    }

    fn deletions(&self) -> Filter {
        Filter::new()
            .kind(Kind::EventDeletion)
            .author(self.public_key())
    }

    fn vanish(&self) -> Filter {
        Filter::new()
            .kind(Kind::RequestToVanish)
            .author(self.public_key())
    }

    fn vanished(&self, events: &[Event]) -> bool {
        events
            .iter()
            .any(|event| drive::vanished(&self.public_key(), event))
    }

    /// Reconciles with every drive relay: takes in and sends what the
    /// extension wrote, downloads what this device lacks, deletions first,
    /// then sends each relay the wraps it lacks and the deletions of wraps
    /// it still holds. Stops before applying anything when a key rotation
    /// retired the drive key.
    pub async fn sync(&mut self) -> Result<SyncReport> {
        let outbox = self.absorb()?;
        let (wraps, deletions, vanish) = (self.wraps(), self.deletions(), self.vanish());
        let wrap_items = self.replica.wrap_items();
        let deletion_items = self.replica.deletion_items();
        let urls = self.relays.urls().to_vec();
        let relays = &self.relays;
        let comparisons = join_all(urls.iter().map(|url| async {
            let deletions = relays.compare(url, &deletions, &deletion_items).await?;
            // A rotation publishes the request to vanish before deleting the
            // old key's wraps: looking after listing the deletions sees it.
            let vanished = relays.events(url, &vanish).await?;
            let wraps = relays.compare(url, &wraps, &wrap_items).await?;
            Ok::<_, Error>((deletions, vanished, wraps))
        }))
        .await;

        let mut report = SyncReport::default();
        let mut synced = Vec::new();
        for (url, comparison) in urls.into_iter().zip(comparisons) {
            match comparison {
                Ok((deletions, vanished, wraps)) => {
                    report.retired |= self.vanished(&vanished);
                    synced.push((url, deletions, wraps));
                }
                Err(e) => report.failed.push((url, e)),
            }
        }
        if report.retired {
            return Ok(report);
        }
        // The comparisons ran before they left: the next sync repairs them.
        let outgoing: HashSet<EventId> = outbox
            .iter()
            .filter_map(|o| o.event.as_ref().map(|event| event.id))
            .collect();
        report.sent = self.send_outbox(outbox).await?;

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
        let full = report.failed.is_empty();
        let newly_synced = full && !self.synced;
        if full {
            self.complete = true;
            self.synced = true;
        }
        self.persist(newly_synced)?;
        (report.lacking, report.repaired) = self.send_repairs(&holdings, &outgoing).await?;
        if !holdings.relays.is_empty()
            && let Ok(latest) = Latest::find(&self.relays, self.public_key()).await
        {
            report.lists = lists::apply(&self.store, latest)?;
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

    /// The outbox keeps its own order: its wraps are not repaired. Returns
    /// the copies relays lacked, and those they took.
    async fn send_repairs(
        &self,
        holdings: &Holdings,
        outgoing: &HashSet<EventId>,
    ) -> Result<(usize, usize)> {
        let unsent = self.store.outbox_wraps()?;
        let mut by_event: HashMap<EventId, Vec<RelayUrl>> = HashMap::new();
        for (relay, id) in self.replica.repairs(holdings) {
            if !unsent.contains(&id) && !outgoing.contains(&id) {
                by_event.entry(id).or_default().push(relay);
            }
        }
        let lacking = by_event.values().map(Vec::len).sum();
        let ids: Vec<EventId> = by_event.keys().copied().collect();
        let events = self.store.events(&ids)?;
        let sent = stream::iter(events)
            .map(|event| {
                let to = &by_event[&event.id];
                async move { self.relays.send(&event, to).await.accepted.len() }
            })
            .buffer_unordered(SEND_CONCURRENCY)
            .fold(0, |total, accepted| async move { total + accepted })
            .await;
        Ok((lacking, sent))
    }

    /// New wraps and deletions as relays receive them. Pass each to
    /// [`receive`](Self::receive). Events missed while disconnected come
    /// with the next [`sync`](Self::sync).
    pub async fn watch(&self) -> Result<impl Stream<Item = Event> + Send + use<>> {
        self.relays
            .subscribe(vec![self.wraps(), self.deletions(), self.vanish()])
            .await
    }

    /// Returns the nodes whose versions changed.
    pub fn receive(&mut self, events: &[Event]) -> Result<BTreeSet<NodeId>> {
        let mut changed = BTreeSet::new();
        for event in events {
            changed.extend(self.replica.insert(&self.keys, event));
        }
        self.persist(false)?;
        Ok(changed)
    }

    /// Takes in the versions the extension and folder sync wrote to the
    /// local database, then sends the outbox to the relays. Returns the
    /// wraps relays took.
    pub async fn take_in(&mut self) -> Result<usize> {
        let outbox = self.absorb()?;
        self.send_outbox(outbox).await
    }

    /// Adds the new versions of the extension and folder sync to the
    /// replica. They wrote the items they show, so only what the tree rules
    /// change besides is written again.
    fn absorb(&mut self) -> Result<Vec<Outgoing>> {
        let outbox = self.store.outbox(0)?;
        let mut changed = BTreeSet::new();
        // Wraps the replica holds change nothing. Sequence numbers start over
        // once the outbox drains, so they cannot tell what is new.
        for event in outbox.iter().filter_map(|o| o.event.as_ref()) {
            changed.extend(self.replica.insert(&self.keys, event));
        }
        for node in &changed {
            match self.store.item(node)? {
                Some(item) => self.items.insert(*node, item),
                None => self.items.remove(node),
            };
        }
        self.persist(false)?;
        Ok(outbox)
    }

    /// In the order written, a wave at a time: a wave ends before each
    /// barrier, so a folder's tombstone leaves only once those of what it
    /// held reached a relay. Stops at the first batch with a wrap no relay
    /// took, which waits for the next try.
    async fn send_outbox(&mut self, outbox: Vec<Outgoing>) -> Result<usize> {
        let mut waves: Vec<Vec<(i64, Option<Event>)>> = Vec::new();
        for outgoing in outbox {
            let entry = (outgoing.seq, outgoing.event);
            match waves.last_mut() {
                Some(wave) if !outgoing.barrier => wave.push(entry),
                _ => waves.push(vec![entry]),
            }
        }
        let mut sent = 0;
        for wave in waves {
            for batch in wave.chunks(SEND_CONCURRENCY) {
                let relays = &self.relays;
                let taken = join_all(batch.iter().cloned().map(|(seq, event)| async move {
                    match event {
                        Some(event) => (seq, !relays.send_all(&event).await.accepted.is_empty()),
                        // Deleted since: nothing left to send.
                        None => (seq, true),
                    }
                }))
                .await;
                let done: Vec<i64> = taken
                    .iter()
                    .filter(|(_, taken)| *taken)
                    .map(|(seq, _)| *seq)
                    .collect();
                self.store.sent(&done)?;
                sent += batch
                    .iter()
                    .filter(|(seq, event)| event.is_some() && done.contains(seq))
                    .count();
                if done.len() < batch.len() {
                    return Ok(sent);
                }
            }
        }
        Ok(sent)
    }

    /// Keeps the local database up to date until `stop` completes: follows
    /// the relays live, and syncs fully at the start and every [`RESYNC`].
    /// Compacts once every drive relay was synced, then every
    /// [`COMPACTION`]. Takes in what the extension wrote whenever `written`
    /// is notified. Fails only when the local database does.
    pub async fn follow(
        &mut self,
        stop: impl Future<Output = ()>,
        written: &Notify,
        mut report: impl FnMut(Followed<'_>),
    ) -> Result<()> {
        let mut stop = pin!(stop);
        let mut live = None;
        loop {
            if live.is_none() {
                live = self.watch().await.ok().map(Box::pin);
            }
            report(Followed::Syncing);
            let synced = self.sync().await?;
            if synced.retired {
                report(Followed::Retired);
                return Ok(());
            }
            report(Followed::Synced(&synced));
            if synced.lists.any() {
                report(Followed::ListsChanged);
            }
            let due = self.compacted.is_none_or(|at| at.elapsed() >= COMPACTION);
            if self.complete && due {
                report(Followed::Compacted(self.compact().await?));
            }
            self.report_items(&mut report);

            let ended = match live.as_mut() {
                Some(events) => {
                    let resync = pin!(task::sleep(RESYNC));
                    self.receive_live(events, &mut stop, written, resync, &mut report)
                        .await?
                }
                None => {
                    let mut retry = pin!(task::sleep(RETRY));
                    loop {
                        tokio::select! {
                            () = &mut stop => break Ended::Stopped,
                            () = &mut retry => break Ended::Closed,
                            () = written.notified() => {
                                self.take_in().await?;
                                self.report_items(&mut report);
                            }
                        }
                    }
                }
            };
            match ended {
                Ended::Stopped => return Ok(()),
                Ended::Resync => {}
                Ended::Closed => live = None,
                Ended::Retired => {
                    report(Followed::Retired);
                    return Ok(());
                }
            }
        }
    }

    async fn receive_live(
        &mut self,
        live: &mut (impl Stream<Item = Event> + Unpin),
        stop: &mut (impl Future<Output = ()> + Unpin),
        written: &Notify,
        mut resync: impl Future<Output = ()> + Unpin,
        report: &mut impl FnMut(Followed<'_>),
    ) -> Result<Ended> {
        loop {
            let first = tokio::select! {
                () = &mut *stop => return Ok(Ended::Stopped),
                () = &mut resync => return Ok(Ended::Resync),
                () = written.notified() => {
                    self.take_in().await?;
                    self.report_items(report);
                    continue;
                }
                event = live.next() => event,
            };
            let Some(first) = first else {
                return Ok(Ended::Closed);
            };
            let mut batch = vec![first];
            let mut window = pin!(task::sleep(BATCH_WINDOW));
            while batch.len() < BATCH_MAX {
                tokio::select! {
                    () = &mut window => break,
                    event = live.next() => match event {
                        Some(event) => batch.push(event),
                        None => break,
                    },
                }
            }
            if self.vanished(&batch) {
                return Ok(Ended::Retired);
            }
            // The request to vanish may have come while disconnected. Every
            // relay must answer: the one that has it may be the one that
            // fails. Otherwise the next sync brings the batch again.
            if batch.iter().any(|event| event.kind == Kind::EventDeletion) {
                let vanish = self.vanish();
                let relays = &self.relays;
                let answers = join_all(relays.urls().iter().map(|url| relays.events(url, &vanish)));
                let mut retired = false;
                let mut answered = true;
                for answer in answers.await {
                    match answer {
                        Ok(found) => retired |= self.vanished(&found),
                        Err(_) => answered = false,
                    }
                }
                if retired {
                    return Ok(Ended::Retired);
                }
                if !answered {
                    continue;
                }
            }
            self.receive(&batch)?;
            self.report_items(report);
        }
    }

    fn report_items(&mut self, report: &mut impl FnMut(Followed<'_>)) {
        if std::mem::take(&mut self.items_changed) {
            report(Followed::ItemsChanged);
        }
    }

    /// Wraps `version` and sends it to every drive relay. Run
    /// `BlobStore::replicate` first on each blob it reuses from an earlier
    /// version: another device may have collected it.
    pub async fn publish(&mut self, version: NodeVersion) -> Result<Published> {
        let (id, event) = self.keys.wrap(&version)?;
        self.replica.insert(&self.keys, &event);
        self.persist(false)?;
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
        modified: Option<i64>,
    ) -> Result<(NodeId, Published)> {
        self.check_parent(parent)?;
        let node = NodeId::random();
        let stamp = self.clock.stamp(self.epoch);
        let mut version = NodeVersion::file(stamp, node, parent, name, key, content);
        version.modified = modified;
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
        let heads = self.replica.heads(node).ok_or(Error::UnknownNode)?;
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
            return Err(Error::UnknownNode);
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
            let heads = self.replica.heads(current).ok_or(Error::UnknownNode)?;
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
        let heads = self.replica.heads(&parent).ok_or(Error::UnknownParent)?;
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
        self.compacted = Some(Instant::now());
        let retention = Retention {
            history: self.store.history()?,
            ..Retention::default()
        };
        let doomed = self.replica.compaction(unix_millis(), &retention);
        let deletions = self.keys.deletions(doomed.iter().copied())?;
        // Kept even if no relay takes them: the next sync sends them.
        for deletion in &deletions {
            self.replica.insert(&self.keys, deletion);
        }
        self.persist(false)?;
        for deletion in &deletions {
            self.relays.send_all(deletion).await;
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
