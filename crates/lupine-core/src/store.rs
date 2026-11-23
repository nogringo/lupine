//! The local database of one drive: what this device knows of it, kept
//! across launches, and the tree as file system integrations show it, with
//! a change log they enumerate from a sync anchor. SQLite in WAL mode, so
//! the app writes while the File Provider extension reads.
//!
//! It holds decrypted metadata, file keys included, like the files a device
//! downloads. The drive secret is not in it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use nostr::event::{Event, EventId};
use nostr::key::PublicKey;
use nostr::types::{RelayUrl, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::ids::{DeviceId, DriveId, NodeId, VersionId};
use crate::items::{Item, ItemChanges};
use crate::node::{DeviceClock, FileMeta, NodeVersion};
use crate::replica::{Record, Snapshot, WrapState};
use crate::tree::EntryKind;

const SCHEMA_VERSION: i32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

const SCHEMA: &str = "
CREATE TABLE drive (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id BLOB NOT NULL,
    pubkey BLOB NOT NULL,
    epoch INTEGER NOT NULL,
    relays TEXT NOT NULL,
    servers TEXT NOT NULL,
    device BLOB NOT NULL,
    clock INTEGER NOT NULL,
    synced INTEGER NOT NULL,
    generation BLOB NOT NULL,
    anchor INTEGER NOT NULL
);
CREATE TABLE wraps (
    id BLOB PRIMARY KEY,
    created_at INTEGER NOT NULL,
    state INTEGER NOT NULL,
    version BLOB,
    event TEXT
) WITHOUT ROWID;
CREATE TABLE versions (
    id BLOB PRIMARY KEY,
    json TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE deletions (
    id BLOB PRIMARY KEY,
    created_at INTEGER NOT NULL,
    event TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE deleted (
    wrap BLOB PRIMARY KEY,
    deletion BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE items (
    node BLOB PRIMARY KEY,
    parent BLOB,
    name TEXT NOT NULL,
    folder INTEGER NOT NULL,
    created INTEGER,
    modified INTEGER,
    file TEXT,
    anchor INTEGER NOT NULL,
    removed INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX items_by_parent ON items (parent, node) WHERE removed = 0;
CREATE INDEX items_by_anchor ON items (anchor);
";

const STATE_VERSION: i64 = 0;
const STATE_UNREADABLE: i64 = 1;
const STATE_JUNK: i64 = 2;
const STATE_DELETED: i64 = 3;

const ITEM_COLUMNS: &str = "node, parent, name, folder, created, modified, file";

/// What the device needs to open the drive, besides its secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveRecord {
    pub id: DriveId,
    pub pubkey: PublicKey,
    pub epoch: u32,
    pub relays: Vec<RelayUrl>,
    /// Most trusted first.
    pub servers: Vec<String>,
}

/// Where a file system integration stands in the change log. Anchors from
/// another database, or from the future, are expired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    generation: [u8; 8],
    seq: u64,
}

impl Anchor {
    pub fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&self.generation);
        bytes[8..].copy_from_slice(&self.seq.to_be_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; 16] = bytes.try_into().ok()?;
        Some(Self {
            generation: bytes[..8].try_into().expect("8 bytes"),
            seq: u64::from_be_bytes(bytes[8..].try_into().expect("8 bytes")),
        })
    }
}

#[derive(Debug)]
pub struct Changes {
    /// Items added or changed since the anchor, as they are now.
    pub updated: Vec<Item>,
    pub removed: Vec<NodeId>,
    pub anchor: Anchor,
    /// The batch stopped at the limit.
    pub more: bool,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrate(conn)
    }

    pub fn memory() -> Result<Self> {
        Self::migrate(Connection::open_in_memory()?)
    }

    fn migrate(mut conn: Connection) -> Result<Self> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                tx.execute_batch(SCHEMA)?;
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            SCHEMA_VERSION => {}
            _ => {
                return Err(Error::Malformed(
                    "the local database is from a later version",
                ));
            }
        }
        tx.commit()?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `None` until a drive is set up.
    pub fn drive(&self) -> Result<Option<DriveRecord>> {
        let row = self
            .lock()
            .query_row(
                "SELECT id, pubkey, epoch, relays, servers FROM drive",
                [],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, pubkey, epoch, relays, servers)) = row else {
            return Ok(None);
        };
        Ok(Some(DriveRecord {
            id: DriveId(array(id)?),
            pubkey: PublicKey::from_slice(&pubkey).map_err(|e| Error::Nostr(e.to_string()))?,
            epoch,
            relays: serde_json::from_str(&relays)?,
            servers: serde_json::from_str(&servers)?,
        }))
    }

    /// Sets up the drive, or updates its lists. The database holds one
    /// drive for its whole life.
    pub fn set_drive(&self, drive: &DriveRecord) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row("SELECT id, pubkey FROM drive", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let relays = serde_json::to_string(&drive.relays)?;
        let servers = serde_json::to_string(&drive.servers)?;
        match existing {
            None => {
                tx.execute(
                    "INSERT INTO drive (singleton, id, pubkey, epoch, relays, servers, device, clock, synced, generation, anchor)
                     VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, 0)",
                    params![
                        drive.id.as_bytes(),
                        drive.pubkey.as_bytes(),
                        drive.epoch,
                        relays,
                        servers,
                        DeviceId::random().as_bytes(),
                        random_bytes::<8>(),
                    ],
                )?;
            }
            Some((id, _)) if id != drive.id.as_bytes() => {
                return Err(Error::Malformed("the local database holds another drive"));
            }
            Some((_, pubkey)) if pubkey != drive.pubkey.as_bytes() => {
                return Err(Error::Malformed("key rotation is not supported yet"));
            }
            Some(_) => {
                tx.execute(
                    "UPDATE drive SET epoch = ?1, relays = ?2, servers = ?3",
                    params![drive.epoch, relays, servers],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The device's clock, and whether every drive relay was synced once.
    pub fn device_state(&self) -> Result<(DeviceClock, bool)> {
        let (device, last, synced) =
            self.lock()
                .query_row("SELECT device, clock, synced FROM drive", [], |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?;
        let clock = DeviceClock {
            device: DeviceId(array(device)?),
            last: last as u64,
        };
        Ok((clock, synced))
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot> {
        let conn = self.lock();
        let mut snapshot = Snapshot::default();

        let mut statement = conn.prepare("SELECT id, created_at, state, version FROM wraps")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let state = match row.get::<_, i64>(2)? {
                STATE_VERSION => WrapState::Version(VersionId(array(row.get(3)?)?)),
                STATE_UNREADABLE => WrapState::Unreadable,
                STATE_JUNK => WrapState::Junk,
                _ => WrapState::Deleted,
            };
            snapshot
                .wraps
                .push((event_id(row.get(0)?)?, timestamp(row.get(1)?), state));
        }

        let mut statement = conn.prepare("SELECT id, json FROM versions")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let version: NodeVersion = serde_json::from_str(&row.get::<_, String>(1)?)?;
            snapshot
                .versions
                .push((VersionId(array(row.get(0)?)?), version));
        }

        let mut statement = conn.prepare("SELECT id, created_at FROM deletions")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            snapshot
                .deletions
                .push((event_id(row.get(0)?)?, timestamp(row.get(1)?)));
        }

        let mut statement = conn.prepare("SELECT wrap, deletion FROM deleted")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            snapshot
                .deleted
                .push((event_id(row.get(0)?)?, event_id(row.get(1)?)?));
        }
        Ok(snapshot)
    }

    /// Writes the replica's journal, the clock and the item changes in one
    /// transaction. Each changed item takes the next anchor.
    pub(crate) fn commit(
        &self,
        records: &[Record],
        clock: DeviceClock,
        synced: bool,
        items: &ItemChanges,
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for record in records {
            write_record(&tx, record)?;
        }
        let mut anchor: u64 = tx.query_row("SELECT anchor FROM drive", [], |row| {
            row.get::<_, i64>(0).map(|a| a as u64)
        })?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO items (node, parent, name, folder, created, modified, file, anchor, removed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)
                 ON CONFLICT (node) DO UPDATE SET
                    parent = excluded.parent, name = excluded.name, folder = excluded.folder,
                    created = excluded.created, modified = excluded.modified, file = excluded.file,
                    anchor = excluded.anchor, removed = 0",
            )?;
            for item in &items.updated {
                anchor += 1;
                let file = item.file.as_ref().map(serde_json::to_string).transpose()?;
                upsert.execute(params![
                    item.node.as_bytes(),
                    item.parent.as_ref().map(NodeId::as_bytes),
                    item.name,
                    item.is_folder(),
                    item.created,
                    item.modified,
                    file,
                    anchor as i64,
                ])?;
            }
            let mut remove =
                tx.prepare_cached("UPDATE items SET removed = 1, anchor = ?2 WHERE node = ?1")?;
            for node in &items.removed {
                anchor += 1;
                remove.execute(params![node.as_bytes(), anchor as i64])?;
            }
        }
        tx.execute(
            "UPDATE drive SET clock = ?1, synced = synced OR ?2, anchor = ?3",
            params![clock.last as i64, synced, anchor as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The stored events among `ids`: wraps relays should get, and
    /// deletions.
    pub(crate) fn events(&self, ids: &[EventId]) -> Result<Vec<Event>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT event FROM wraps WHERE id = ?1 AND event IS NOT NULL
             UNION ALL SELECT event FROM deletions WHERE id = ?1",
        )?;
        let mut events = Vec::new();
        for id in ids {
            let json: Option<String> = statement
                .query_row([id.as_bytes()], |row| row.get(0))
                .optional()?;
            if let Some(json) = json {
                events.push(Event::from_json(json).map_err(|e| Error::Nostr(e.to_string()))?);
            }
        }
        Ok(events)
    }

    /// Every item shown, for the session to diff against.
    pub(crate) fn shown_items(&self) -> Result<HashMap<NodeId, Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM items WHERE removed = 0"
        ))?;
        let items = collect_items(statement.query([])?)?;
        Ok(items.into_iter().map(|item| (item.node, item)).collect())
    }

    pub fn item(&self, node: &NodeId) -> Result<Option<Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items WHERE node = ?1 AND removed = 0"
        ))?;
        Ok(collect_items(statement.query([node.as_bytes()])?)?.pop())
    }

    /// The items in `parent` (`None` for the drive root) after `after`, by
    /// node id.
    pub fn children(
        &self,
        parent: Option<NodeId>,
        after: Option<NodeId>,
        limit: usize,
    ) -> Result<Vec<Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items
             WHERE parent IS ?1 AND removed = 0 AND node > ?2 ORDER BY node LIMIT ?3"
        ))?;
        let rows = statement.query(params![
            parent.as_ref().map(NodeId::as_bytes),
            after.map(|n| n.0.to_vec()).unwrap_or_default(),
            limit as i64,
        ])?;
        collect_items(rows)
    }

    /// Every item after `after`, by node id.
    pub fn items(&self, after: Option<NodeId>, limit: usize) -> Result<Vec<Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items WHERE removed = 0 AND node > ?1 ORDER BY node LIMIT ?2"
        ))?;
        let rows = statement.query(params![
            after.map(|n| n.0.to_vec()).unwrap_or_default(),
            limit as i64
        ])?;
        collect_items(rows)
    }

    pub fn anchor(&self) -> Result<Anchor> {
        current_anchor(&self.lock())
    }

    /// What changed after `since`, at most `limit` items. Fails with
    /// [`Error::AnchorExpired`] for an anchor this database did not give.
    pub fn changes(&self, since: &[u8], limit: usize) -> Result<Changes> {
        let limit = limit.max(1);
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let current = current_anchor(&tx)?;
        let since = Anchor::from_bytes(since)
            .filter(|a| a.generation == current.generation && a.seq <= current.seq)
            .ok_or(Error::AnchorExpired)?;

        let mut statement = tx.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS}, anchor, removed FROM items WHERE anchor > ?1 ORDER BY anchor LIMIT ?2"
        ))?;
        let mut rows = statement.query(params![since.seq as i64, limit as i64 + 1])?;
        let mut changes = Changes {
            updated: Vec::new(),
            removed: Vec::new(),
            anchor: current,
            more: false,
        };
        let mut last = since.seq;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            if count == limit {
                changes.more = true;
                changes.anchor.seq = last;
                break;
            }
            count += 1;
            last = row.get::<_, i64>(7)? as u64;
            if row.get::<_, bool>(8)? {
                changes.removed.push(NodeId(array(row.get(0)?)?));
            } else {
                changes.updated.push(read_item(row)?);
            }
        }
        Ok(changes)
    }
}

fn write_record(tx: &rusqlite::Transaction<'_>, record: &Record) -> Result<()> {
    match record {
        Record::Wrap {
            id,
            created_at,
            state,
            event,
        } => {
            let (code, version) = match state {
                WrapState::Version(version) => (STATE_VERSION, Some(version.as_bytes())),
                WrapState::Unreadable => (STATE_UNREADABLE, None),
                WrapState::Junk => (STATE_JUNK, None),
                WrapState::Deleted => (STATE_DELETED, None),
            };
            tx.prepare_cached(
                "INSERT INTO wraps (id, created_at, state, version, event) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                    state = excluded.state, version = excluded.version, event = excluded.event",
            )?
            .execute(params![
                id.as_bytes(),
                created_at.as_secs() as i64,
                code,
                version,
                event.as_ref().map(Event::as_json),
            ])?;
        }
        Record::Version { id, version } => {
            tx.prepare_cached("INSERT OR IGNORE INTO versions (id, json) VALUES (?1, ?2)")?
                .execute(params![id.as_bytes(), serde_json::to_string(version)?])?;
        }
        Record::VersionGone(id) => {
            tx.prepare_cached("DELETE FROM versions WHERE id = ?1")?
                .execute([id.as_bytes()])?;
        }
        Record::Deletion(event) => {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO deletions (id, created_at, event) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![
                event.id.as_bytes(),
                event.created_at.as_secs() as i64,
                event.as_json(),
            ])?;
        }
        Record::Deleted { wrap, by } => {
            tx.prepare_cached("INSERT OR IGNORE INTO deleted (wrap, deletion) VALUES (?1, ?2)")?
                .execute(params![wrap.as_bytes(), by.as_bytes()])?;
        }
    }
    Ok(())
}

fn current_anchor(conn: &Connection) -> Result<Anchor> {
    let (generation, seq) = conn
        .query_row("SELECT generation, anchor FROM drive", [], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()?
        .ok_or(Error::Malformed("no drive in the local database"))?;
    Ok(Anchor {
        generation: array(generation)?,
        seq: seq as u64,
    })
}

fn collect_items(mut rows: rusqlite::Rows<'_>) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(read_item(row)?);
    }
    Ok(items)
}

fn read_item(row: &Row<'_>) -> Result<Item> {
    let parent: Option<Vec<u8>> = row.get(1)?;
    let file: Option<String> = row.get(6)?;
    let folder: bool = row.get(3)?;
    Ok(Item {
        node: NodeId(array(row.get(0)?)?),
        parent: parent.map(array).transpose()?.map(NodeId),
        name: row.get(2)?,
        kind: if folder {
            EntryKind::Folder
        } else {
            EntryKind::File
        },
        created: row.get(4)?,
        modified: row.get(5)?,
        file: file
            .map(|json| serde_json::from_str::<FileMeta>(&json))
            .transpose()?,
    })
}

fn array<const N: usize>(bytes: Vec<u8>) -> Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| Error::Malformed("corrupt local database"))
}

fn event_id(bytes: Vec<u8>) -> Result<EventId> {
    Ok(EventId::from_byte_array(array(bytes)?))
}

fn timestamp(secs: i64) -> Timestamp {
    Timestamp::from_secs(secs.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;

    fn record(id: u8) -> DriveRecord {
        DriveRecord {
            id: DriveId([id; 16]),
            pubkey: Keys::generate().public_key(),
            epoch: 0,
            relays: vec![RelayUrl::parse("wss://relay.example.com").unwrap()],
            servers: vec!["https://blossom.example.com".into()],
        }
    }

    #[test]
    fn a_database_holds_one_drive_for_life() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Drives/drive.sqlite");
        let store = Store::open(&path).unwrap();
        assert!(store.drive().unwrap().is_none());

        let mut drive = record(1);
        store.set_drive(&drive).unwrap();
        let (clock, synced) = store.device_state().unwrap();
        assert!(!synced);

        drive.servers.push("https://other.example.com".into());
        store.set_drive(&drive).unwrap();
        assert!(store.set_drive(&record(2)).is_err());
        drop(store);

        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.drive().unwrap(), Some(drive));
        assert_eq!(reopened.device_state().unwrap().0.device, clock.device);
    }
}
