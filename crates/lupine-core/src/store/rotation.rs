//! Key rotation in the local database: the rotation this device started,
//! the move of the database to the new key, and the wraps of the key a
//! rotation retired, still to delete.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use nostr::event::{Event, EventId};
use nostr::key::PublicKey;
use nostr::types::RelayUrl;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{
    DescriptorRecord, DriveRecord, STATE_DELETED, STATE_UNREADABLE, STATE_VERSION, Store, array,
    event_id,
};
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::ids::VersionId;
use crate::node::NodeVersion;
use crate::replica::WrapState;

/// A key rotation this device started: what it needs to finish it without
/// the user's key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rotation {
    pub epoch: u32,
    pub pubkey: PublicKey,
    /// The new descriptor, signed by the user.
    pub descriptor: Event,
    /// The user's deletion of the older descriptors.
    pub deletion: Option<Event>,
    pub name: String,
    /// The user's write relays.
    pub user_relays: Vec<RelayUrl>,
    /// Set before the descriptor is published: from then on the rotation
    /// can only be finished.
    pub committed: bool,
    /// The old key's request to vanish, once signed.
    pub marker: Option<Event>,
}

/// The move of this database to a new drive key.
pub struct Move<'a> {
    pub old: &'a DriveKeys,
    pub new: &'a DriveKeys,
    /// The drive under the new key.
    pub drive: &'a DriveRecord,
    pub descriptor: Option<&'a DescriptorRecord>,
    /// The old key's request to vanish, published before its wraps are
    /// deleted.
    pub marker: &'a Event,
}

impl Store {
    pub fn rotation(&self) -> Result<Option<Rotation>> {
        let json: Option<Option<String>> = self
            .lock()
            .query_row("SELECT rotation FROM drive", [], |row| row.get(0))
            .optional()?;
        Ok(json
            .flatten()
            .map(|json| serde_json::from_str(&json))
            .transpose()?)
    }

    pub fn set_rotation(&self, rotation: Option<&Rotation>) -> Result<()> {
        let json = rotation.map(serde_json::to_string).transpose()?;
        let updated = self
            .lock()
            .execute("UPDATE drive SET rotation = ?1", [json])?;
        if updated == 0 {
            return Err(Error::Malformed("no drive in the local database"));
        }
        Ok(())
    }

    /// The wraps that carry something to keep, by id after `after`: one
    /// event per wrap, with what it carries.
    pub(crate) fn kept_wraps(
        &self,
        after: Option<EventId>,
        limit: usize,
    ) -> Result<Vec<(WrapState, Event)>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT state, version, event FROM wraps
             WHERE id > ?1 AND event IS NOT NULL AND state IN (?2, ?3)
             ORDER BY id LIMIT ?4",
        )?;
        let after = after.map(|id| id.as_bytes().to_vec()).unwrap_or_default();
        let mut rows = statement.query(params![
            after,
            STATE_VERSION,
            STATE_UNREADABLE,
            limit as i64
        ])?;
        let mut wraps = Vec::new();
        while let Some(row) = rows.next()? {
            let state = match row.get::<_, Option<Vec<u8>>>(1)? {
                Some(version) if row.get::<_, i64>(0)? == STATE_VERSION => {
                    WrapState::Version(VersionId(array(version)?))
                }
                _ => WrapState::Unreadable,
            };
            wraps.push((state, parse_event(row.get(2)?)?));
        }
        Ok(wraps)
    }

    /// The wraps of a later format.
    pub(crate) fn unreadable_events(&self) -> Result<Vec<Event>> {
        let conn = self.lock();
        let mut statement =
            conn.prepare("SELECT event FROM wraps WHERE state = ?1 AND event IS NOT NULL")?;
        let mut rows = statement.query([STATE_UNREADABLE])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(parse_event(row.get(0)?)?);
        }
        Ok(events)
    }

    /// How many wraps [`kept_wraps`](Self::kept_wraps) lists.
    pub(crate) fn kept_count(&self) -> Result<usize> {
        let count: i64 = self.lock().query_row(
            "SELECT count(*) FROM wraps WHERE event IS NOT NULL AND state IN (?1, ?2)",
            [STATE_VERSION, STATE_UNREADABLE],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// The key a rotation retired, and its request to vanish, until every
    /// wrap addressed to it is deleted.
    pub fn retiring(&self) -> Result<Option<(PublicKey, Event)>> {
        let row: Option<(Option<Vec<u8>>, Option<String>)> = self
            .lock()
            .query_row("SELECT retiring, marker FROM drive", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let Some((Some(pubkey), Some(marker))) = row else {
            return Ok(None);
        };
        let pubkey = PublicKey::from_slice(&pubkey).map_err(|e| Error::Nostr(e.to_string()))?;
        Ok(Some((pubkey, parse_event(marker)?)))
    }

    /// Wraps of the retired key still to delete.
    pub(crate) fn retired_wraps(&self, limit: usize) -> Result<Vec<EventId>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached("SELECT wrap FROM retired LIMIT ?1")?;
        let mut rows = statement.query([limit as i64])?;
        let mut wraps = Vec::new();
        while let Some(row) = rows.next()? {
            wraps.push(event_id(row.get(0)?)?);
        }
        Ok(wraps)
    }

    /// Relays took the deletion of `wraps`. Once none is left, the key is
    /// retired.
    pub(crate) fn deleted_retired(&self, wraps: &[EventId]) -> Result<bool> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut delete = tx.prepare_cached("DELETE FROM retired WHERE wrap = ?1")?;
            for wrap in wraps {
                delete.execute([wrap.as_bytes()])?;
            }
        }
        let left: i64 = tx.query_row("SELECT count(*) FROM retired", [], |row| row.get(0))?;
        if left == 0 {
            tx.execute("UPDATE drive SET retiring = NULL, marker = NULL", [])?;
        }
        tx.commit()?;
        Ok(left == 0)
    }

    /// Gives up deleting the retired key's wraps, when its secret is lost.
    pub fn forget_retirement(&self) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM retired", [])?;
        tx.execute("UPDATE drive SET retiring = NULL, marker = NULL", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Moves this database to the new key, from `scratch`, the database of
    /// a full sync with it: its events replace those of the old key, which
    /// wait in `retired` to be deleted. The versions only this database
    /// holds, such as changes it has not sent, are wrapped again for the
    /// new key, unless a version of `scratch` dominates them, and the
    /// outbox keeps their order. Items, synced folders, device ids and
    /// settings stay. Returns the versions wrapped again.
    pub(crate) fn transplant(&self, scratch: &Path, to: Move<'_>) -> Result<usize> {
        if to.drive.pubkey != to.new.public_key() {
            return Err(Error::Malformed("the drive record is not the new key's"));
        }
        let path = scratch
            .to_str()
            .ok_or(Error::Malformed("the scratch database path is not UTF-8"))?;
        let mut conn = self.lock();
        conn.execute("ATTACH DATABASE ?1 AS scratch", [path])?;
        let moved = move_to_new_key(&mut conn, &to);
        conn.execute("DETACH DATABASE scratch", [])?;
        moved
    }
}

fn move_to_new_key(conn: &mut Connection, to: &Move<'_>) -> Result<usize> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: Vec<u8> = tx.query_row("SELECT pubkey FROM main.drive", [], |row| row.get(0))?;
    if current != to.old.public_key().as_bytes() {
        return Err(Error::DriveKeyChanged);
    }
    let scratch_drive: Vec<u8> =
        tx.query_row("SELECT pubkey FROM scratch.drive", [], |row| row.get(0))?;
    if scratch_drive != to.new.public_key().as_bytes() {
        return Err(Error::Malformed("the scratch database holds another key"));
    }

    let late = late_versions(&tx, to)?;
    let late_unreadable = late_unreadable(&tx, to)?;
    let outbox = outbox_versions(&tx)?;

    tx.execute("DELETE FROM main.retired", [])?;
    tx.execute(
        "INSERT INTO main.retired (wrap) SELECT id FROM main.wraps WHERE state != ?1",
        [STATE_DELETED],
    )?;
    tx.execute_batch(
        "DELETE FROM main.wraps;
         DELETE FROM main.deletions;
         DELETE FROM main.deleted;
         DELETE FROM main.outbox;
         INSERT INTO main.wraps (id, created_at, state, version, event)
            SELECT id, created_at, state, version, event FROM scratch.wraps;
         INSERT INTO main.deletions (id, created_at, event)
            SELECT id, created_at, event FROM scratch.deletions;
         INSERT INTO main.deleted (wrap, deletion) SELECT wrap, deletion FROM scratch.deleted;
         INSERT OR IGNORE INTO main.versions (id, node, json)
            SELECT id, node, json FROM scratch.versions;",
    )?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO main.wraps (id, created_at, state, version, event)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (version, event) in &late {
            insert.execute(params![
                event.id.as_bytes(),
                event.created_at.as_secs() as i64,
                STATE_VERSION,
                version.as_bytes(),
                event.as_json(),
            ])?;
        }
        for event in &late_unreadable {
            insert.execute(params![
                event.id.as_bytes(),
                event.created_at.as_secs() as i64,
                STATE_UNREADABLE,
                None::<Vec<u8>>,
                event.as_json(),
            ])?;
        }
        // A dropped wrap passes its barrier on, so waves keep their order.
        let mut queue =
            tx.prepare_cached("INSERT INTO main.outbox (seq, wrap, barrier) VALUES (?1, ?2, ?3)")?;
        let rewrapped: HashMap<VersionId, &Event> = late
            .iter()
            .map(|(version, event)| (*version, event))
            .collect();
        let mut queued = HashSet::new();
        let mut barrier = false;
        for (seq, wrap_barrier, version) in outbox {
            barrier |= wrap_barrier;
            let Some((version, event)) = version.and_then(|v| Some((v, *rewrapped.get(&v)?)))
            else {
                continue;
            };
            if queued.insert(version) {
                queue.execute(params![seq, event.id.as_bytes(), barrier])?;
                barrier = false;
            }
        }
        // Sent to the old key already, in waves this order cannot tell: one
        // at a time, in the order written, so no folder's tombstone leaves
        // before those of what it held.
        let mut append =
            tx.prepare_cached("INSERT INTO main.outbox (wrap, barrier) VALUES (?1, 1)")?;
        for (version, event) in &late {
            if !queued.contains(version) {
                append.execute([event.id.as_bytes()])?;
            }
        }
    }
    tx.execute(
        "DELETE FROM main.versions WHERE id NOT IN
            (SELECT version FROM main.wraps WHERE state = ?1 AND version IS NOT NULL)",
        [STATE_VERSION],
    )?;
    let drive = to.drive;
    tx.execute(
        "UPDATE main.drive SET pubkey = ?1, epoch = ?2, relays = ?3, servers = ?4,
            relays_at = 0, servers_at = 0, synced = 1, rotation = NULL,
            retiring = ?5, marker = ?6",
        params![
            drive.pubkey.as_bytes(),
            drive.epoch,
            serde_json::to_string(&drive.relays)?,
            serde_json::to_string(&drive.servers)?,
            to.old.public_key().as_bytes(),
            to.marker.as_json(),
        ],
    )?;
    match to.descriptor {
        Some(descriptor) => {
            tx.execute(
                "UPDATE main.drive SET descriptor = ?1, name = ?2, user_relays = ?3",
                params![
                    descriptor.event.as_json(),
                    descriptor.name,
                    serde_json::to_string(&descriptor.user_relays)?,
                ],
            )?;
        }
        // The old one names the old key: republishing it would undo its
        // deletion.
        None => {
            tx.execute("UPDATE main.drive SET descriptor = NULL", [])?;
        }
    }
    tx.commit()?;
    Ok(late.len() + late_unreadable.len())
}

/// The versions this database holds that no wrap of `scratch` carries,
/// and no version of `scratch` dominates, wrapped again for the new key,
/// in the order written.
fn late_versions(tx: &Transaction<'_>, to: &Move<'_>) -> Result<Vec<(VersionId, Event)>> {
    let mut statement = tx.prepare(
        "SELECT id, node, json FROM main.versions WHERE id NOT IN
            (SELECT version FROM scratch.wraps WHERE state = ?1 AND version IS NOT NULL)",
    )?;
    let mut rows = statement.query([STATE_VERSION])?;
    let mut candidates = Vec::new();
    while let Some(row) = rows.next()? {
        let version: NodeVersion = serde_json::from_str(&row.get::<_, String>(2)?)?;
        candidates.push((
            VersionId(array(row.get(0)?)?),
            row.get::<_, Vec<u8>>(1)?,
            version,
        ));
    }
    candidates.sort_by_key(|(_, _, version)| (version.time, version.seq()));
    let mut on_node = tx.prepare_cached("SELECT json FROM scratch.versions WHERE node = ?1")?;
    let mut carrier = tx.prepare_cached(
        "SELECT event FROM main.wraps
         WHERE state = ?1 AND version = ?2 AND event IS NOT NULL LIMIT 1",
    )?;
    let mut late = Vec::new();
    for (id, node, version) in candidates {
        let mut known = on_node.query([&node])?;
        let mut dominated = false;
        while let Some(row) = known.next()? {
            let other: NodeVersion = serde_json::from_str(&row.get::<_, String>(0)?)?;
            dominated |= other.dominates(&version);
        }
        if dominated {
            continue;
        }
        let event: Option<String> = carrier
            .query_row(params![STATE_VERSION, id.as_bytes()], |row| row.get(0))
            .optional()?;
        let Some(event) = event else {
            continue;
        };
        if let Some((rewrapped, event)) = to.new.rewrap(to.old, &parse_event(event)?)
            && rewrapped == id
        {
            late.push((id, event));
        }
    }
    Ok(late)
}

/// Wraps of a later format that `scratch` lacks, wrapped again.
fn late_unreadable(tx: &Transaction<'_>, to: &Move<'_>) -> Result<Vec<Event>> {
    let unreadable = |table: &str| -> Result<Vec<Event>> {
        let mut statement = tx.prepare(&format!(
            "SELECT event FROM {table}.wraps WHERE state = ?1 AND event IS NOT NULL"
        ))?;
        let mut rows = statement.query([STATE_UNREADABLE])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(parse_event(row.get(0)?)?);
        }
        Ok(events)
    };
    let held: HashSet<VersionId> = unreadable("scratch")?
        .iter()
        .filter_map(|event| to.new.plaintext(event))
        .map(|plaintext| VersionId::of(&plaintext))
        .collect();
    let mut late = Vec::new();
    let mut seen = HashSet::new();
    for event in unreadable("main")? {
        if let Some((id, rewrapped)) = to.new.rewrap(to.old, &event)
            && !held.contains(&id)
            && seen.insert(id)
        {
            late.push(rewrapped);
        }
    }
    Ok(late)
}

/// The outbox in order: each wrap's sequence number, barrier and version.
fn outbox_versions(tx: &Transaction<'_>) -> Result<Vec<(i64, bool, Option<VersionId>)>> {
    let mut statement = tx.prepare(
        "SELECT outbox.seq, outbox.barrier, wraps.version FROM main.outbox
         LEFT JOIN main.wraps ON wraps.id = outbox.wrap AND wraps.state = ?1
         ORDER BY outbox.seq",
    )?;
    let mut rows = statement.query([STATE_VERSION])?;
    let mut outbox = Vec::new();
    while let Some(row) = rows.next()? {
        let version = row
            .get::<_, Option<Vec<u8>>>(2)?
            .map(array)
            .transpose()?
            .map(VersionId);
        outbox.push((row.get(0)?, row.get(1)?, version));
    }
    Ok(outbox)
}

fn parse_event(json: String) -> Result<Event> {
    Event::from_json(json).map_err(|e| Error::Nostr(e.to_string()))
}
