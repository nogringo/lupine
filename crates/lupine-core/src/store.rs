//! The local database of one drive: what this device knows of it, kept
//! across launches, and the tree as file system integrations show it, with
//! a change log they enumerate from a sync anchor. SQLite in WAL mode, so
//! the app and the File Provider extension share it. The extension and
//! folder sync write their own versions and leave their wraps in an outbox,
//! which the app's session sends.
//!
//! It holds decrypted metadata, file keys included, like the files a device
//! downloads. The drive secret is not in it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use nostr::event::{Event, EventId};
use nostr::key::PublicKey;
use nostr::types::{RelayUrl, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::ids::{DeviceId, DriveId, NodeId, Sha256Hash, VersionId};
use crate::items::{Item, ItemChanges};
use crate::node::{DeviceClock, FileMeta, NodeVersion};
use crate::replica::{Record, Retention, Snapshot, WrapState};
use crate::tree::EntryKind;

mod blobs;
mod folders;
mod lists;

pub use lists::ListsState;

const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// Migration `n` takes the schema from version `n` to `n + 1`. A new
/// database runs them all.
const MIGRATIONS: [&str; 7] = [
    SCHEMA_V1,
    MIGRATION_V2,
    MIGRATION_V3,
    MIGRATION_V4,
    MIGRATION_V5,
    MIGRATION_V6,
    MIGRATION_V7,
];

const SCHEMA_V1: &str = "
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

/// The File Provider extension writes versions too, under a device id of
/// its own, and leaves their wraps in the outbox for the session to send.
const MIGRATION_V2: &str = "
ALTER TABLE versions ADD COLUMN node BLOB;
UPDATE versions SET node = unhex(json ->> '$.node');
CREATE INDEX versions_by_node ON versions (node);
CREATE TABLE writers (
    writer INTEGER PRIMARY KEY,
    device BLOB NOT NULL,
    clock INTEGER NOT NULL
);
INSERT INTO writers (writer, device, clock) SELECT 0, device, clock FROM drive;
INSERT INTO writers (writer, device, clock) SELECT 1, randomblob(16), 0 FROM drive;
ALTER TABLE drive DROP COLUMN device;
ALTER TABLE drive DROP COLUMN clock;
CREATE TABLE outbox (
    seq INTEGER PRIMARY KEY,
    wrap BLOB NOT NULL,
    barrier INTEGER NOT NULL
);
";

/// Folder sync: the synced folders, what each entry was when both sides
/// last agreed, and a device id of its own.
const MIGRATION_V3: &str = "
INSERT INTO writers (writer, device, clock) SELECT 2, randomblob(16), 0 FROM drive;
CREATE TABLE folders (
    id INTEGER PRIMARY KEY,
    root BLOB NOT NULL,
    path TEXT NOT NULL,
    file_id INTEGER NOT NULL,
    bookmark BLOB
);
CREATE TABLE synced (
    folder INTEGER NOT NULL,
    node BLOB NOT NULL,
    parent BLOB NOT NULL,
    name TEXT NOT NULL,
    is_folder INTEGER NOT NULL,
    content BLOB,
    modified INTEGER,
    file_id INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    PRIMARY KEY (folder, node)
) WITHOUT ROWID;
";

/// Sign-in: the descriptor this device read, to republish it, and the
/// user's relays it came from.
const MIGRATION_V4: &str = "
ALTER TABLE drive ADD COLUMN descriptor TEXT;
ALTER TABLE drive ADD COLUMN name TEXT;
ALTER TABLE drive ADD COLUMN user_relays TEXT;
";

/// The history window this device compacts with, in seconds. `NULL` for
/// the default.
const MIGRATION_V5: &str = "
ALTER TABLE drive ADD COLUMN history INTEGER;
";

/// Editing the drive's lists: when this device applied them, the servers
/// the drive leaves, and the fill that copies every blob to the others.
const MIGRATION_V6: &str = "
ALTER TABLE drive ADD COLUMN relays_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drive ADD COLUMN servers_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drive ADD COLUMN leaving TEXT NOT NULL DEFAULT '[]';
ALTER TABLE drive ADD COLUMN fill INTEGER NOT NULL DEFAULT 0;
";

/// Blob collection: each blob this device knows of, with the Unix
/// milliseconds since which no version it holds references it (`NULL` while
/// one does), and the blobs each chunk table lists.
const MIGRATION_V7: &str = "
CREATE TABLE blobs (
    hash BLOB PRIMARY KEY,
    unreferenced INTEGER
) WITHOUT ROWID;
CREATE TABLE tables (
    hash BLOB PRIMARY KEY,
    chunks BLOB NOT NULL
) WITHOUT ROWID;
";

const STATE_VERSION: i64 = 0;
const STATE_UNREADABLE: i64 = 1;
const STATE_JUNK: i64 = 2;
const STATE_DELETED: i64 = 3;

const ITEM_COLUMNS: &str = "node, parent, name, folder, created, modified, file";

/// The processes of this install that write versions. Each has its own
/// device id, since version vectors take a device's versions to follow one
/// another, each written after seeing the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    /// The drive session, which syncs with the relays.
    Session,
    /// A file system integration in a process of its own, such as a File
    /// Provider extension.
    Extension,
    /// Folder sync, in the app's process but beside the session.
    Folders,
}

impl Writer {
    const ALL: [Self; 3] = [Self::Session, Self::Extension, Self::Folders];

    fn code(self) -> i64 {
        match self {
            Self::Session => 0,
            Self::Extension => 1,
            Self::Folders => 2,
        }
    }
}

/// A wrap the extension or folder sync wrote, for the session to send.
#[derive(Debug)]
pub(crate) struct Outgoing {
    pub seq: i64,
    /// Sent only once every earlier wrap was.
    pub barrier: bool,
    /// `None` once the wrap was deleted: there is nothing left to send.
    pub event: Option<Event>,
}

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

/// The descriptor a device signed in with. A drive opened with a recovery
/// key has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorRecord {
    pub event: Event,
    pub name: String,
    /// The user's write relays, where every device republishes it.
    pub user_relays: Vec<RelayUrl>,
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
        let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let applied = MIGRATIONS.get(version as usize..).ok_or(Error::Malformed(
            "the local database is from a later version",
        ))?;
        for migration in applied {
            tx.execute_batch(migration)?;
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as u32)?;
        tx.commit()?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
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
                    "INSERT INTO drive (singleton, id, pubkey, epoch, relays, servers, synced, generation, anchor)
                     VALUES (1, ?1, ?2, ?3, ?4, ?5, 0, ?6, 0)",
                    params![
                        drive.id.as_bytes(),
                        drive.pubkey.as_bytes(),
                        drive.epoch,
                        relays,
                        servers,
                        random_bytes::<8>(),
                    ],
                )?;
                for writer in Writer::ALL {
                    tx.execute(
                        "INSERT INTO writers (writer, device, clock) VALUES (?1, ?2, 0)",
                        params![writer.code(), DeviceId::random().as_bytes()],
                    )?;
                }
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

    pub fn set_descriptor(&self, descriptor: &DescriptorRecord) -> Result<()> {
        let updated = self.lock().execute(
            "UPDATE drive SET descriptor = ?1, name = ?2, user_relays = ?3",
            params![
                descriptor.event.as_json(),
                descriptor.name,
                serde_json::to_string(&descriptor.user_relays)?,
            ],
        )?;
        if updated == 0 {
            return Err(Error::Malformed("no drive in the local database"));
        }
        Ok(())
    }

    pub fn descriptor(&self) -> Result<Option<DescriptorRecord>> {
        let row = self
            .lock()
            .query_row(
                "SELECT descriptor, name, user_relays FROM drive WHERE descriptor IS NOT NULL",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((event, name, user_relays)) = row else {
            return Ok(None);
        };
        Ok(Some(DescriptorRecord {
            event: Event::from_json(event).map_err(|e| Error::Nostr(e.to_string()))?,
            name,
            user_relays: serde_json::from_str(&user_relays)?,
        }))
    }

    /// How long superseded versions stay before compaction: a setting of
    /// this device.
    pub fn history(&self) -> Result<Duration> {
        let secs: Option<Option<i64>> = self
            .lock()
            .query_row("SELECT history FROM drive", [], |row| row.get(0))
            .optional()?;
        Ok(match secs.flatten() {
            Some(secs) => Duration::from_secs(secs.max(0) as u64),
            None => Retention::default().history,
        })
    }

    pub fn set_history(&self, window: Duration) -> Result<()> {
        let secs = i64::try_from(window.as_secs()).unwrap_or(i64::MAX);
        let updated = self
            .lock()
            .execute("UPDATE drive SET history = ?1", [secs])?;
        if updated == 0 {
            return Err(Error::Malformed("no drive in the local database"));
        }
        Ok(())
    }

    /// Wraps the extension and folder sync wrote that no relay took yet.
    pub fn unsent(&self) -> Result<u64> {
        let count: i64 = self.lock().query_row(
            "SELECT count(*) FROM outbox JOIN wraps ON wraps.id = outbox.wrap
             WHERE wraps.event IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// The session's clock, and whether every drive relay was synced once.
    pub fn device_state(&self) -> Result<(DeviceClock, bool)> {
        let conn = self.lock();
        let synced = conn.query_row("SELECT synced FROM drive", [], |row| row.get(0))?;
        Ok((writer_clock(&conn, Writer::Session)?, synced))
    }

    /// The clock `writer` numbers its versions with.
    pub fn clock(&self, writer: Writer) -> Result<DeviceClock> {
        writer_clock(&self.lock(), writer)
    }

    /// The versions of `node` that some wrap not deleted still carries.
    pub fn node_versions(&self, node: &NodeId) -> Result<Vec<(VersionId, NodeVersion)>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached("SELECT id, json FROM versions WHERE node = ?1")?;
        let mut rows = statement.query([node.as_bytes()])?;
        let mut versions = Vec::new();
        while let Some(row) = rows.next()? {
            let version: NodeVersion = serde_json::from_str(&row.get::<_, String>(1)?)?;
            versions.push((VersionId(array(row.get(0)?)?), version));
        }
        Ok(versions)
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
        write_items(&tx, items)?;
        set_clock(&tx, Writer::Session, clock)?;
        tx.execute("UPDATE drive SET synced = synced OR ?1", [synced])?;
        tx.commit()?;
        Ok(())
    }

    /// What a writer other than the session wrote, in one transaction: the
    /// records, its clock, the items they change, `outbox`, the wraps for
    /// the session to send, in order, each with its barrier, and the blobs
    /// it uploaded that a version now references.
    pub(crate) fn commit_outbox(
        &self,
        writer: Writer,
        records: &[Record],
        clock: DeviceClock,
        items: &ItemChanges,
        outbox: &[(EventId, bool)],
        referenced: &[Sha256Hash],
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for record in records {
            write_record(&tx, record)?;
        }
        write_items(&tx, items)?;
        set_clock(&tx, writer, clock)?;
        {
            let mut insert =
                tx.prepare_cached("INSERT INTO outbox (wrap, barrier) VALUES (?1, ?2)")?;
            for (wrap, barrier) in outbox {
                insert.execute(params![wrap.as_bytes(), barrier])?;
            }
            let mut referenced_now =
                tx.prepare_cached("UPDATE blobs SET unreferenced = NULL WHERE hash = ?1")?;
            for hash in referenced {
                referenced_now.execute([hash.as_bytes()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The outbox after `seq`, in the order written.
    pub(crate) fn outbox(&self, after: i64) -> Result<Vec<Outgoing>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT outbox.seq, outbox.barrier, wraps.event FROM outbox
             LEFT JOIN wraps ON wraps.id = outbox.wrap
             WHERE outbox.seq > ?1 ORDER BY outbox.seq",
        )?;
        let mut rows = statement.query([after])?;
        let mut outgoing = Vec::new();
        while let Some(row) = rows.next()? {
            let event = row
                .get::<_, Option<String>>(2)?
                .map(Event::from_json)
                .transpose()
                .map_err(|e| Error::Nostr(e.to_string()))?;
            outgoing.push(Outgoing {
                seq: row.get(0)?,
                barrier: row.get(1)?,
                event,
            });
        }
        Ok(outgoing)
    }

    /// The wraps still in the outbox.
    pub(crate) fn outbox_wraps(&self) -> Result<HashSet<EventId>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached("SELECT wrap FROM outbox")?;
        let mut rows = statement.query([])?;
        let mut wraps = HashSet::new();
        while let Some(row) = rows.next()? {
            wraps.insert(event_id(row.get(0)?)?);
        }
        Ok(wraps)
    }

    /// Takes `seqs` out of the outbox.
    pub(crate) fn sent(&self, seqs: &[i64]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut delete = tx.prepare_cached("DELETE FROM outbox WHERE seq = ?1")?;
            for seq in seqs {
                delete.execute([seq])?;
            }
        }
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

    /// The item named exactly `name` in `parent`, `None` for the drive root.
    pub fn child_named(&self, parent: Option<NodeId>, name: &str) -> Result<Option<Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items
             WHERE parent IS ?1 AND name = ?2 AND removed = 0 ORDER BY node LIMIT 1"
        ))?;
        let rows = statement.query(params![parent.as_ref().map(NodeId::as_bytes), name])?;
        Ok(collect_items(rows)?.pop())
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
            tx.prepare_cached(
                "INSERT OR IGNORE INTO versions (id, node, json) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![
                id.as_bytes(),
                version.node.as_bytes(),
                serde_json::to_string(version)?
            ])?;
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

/// Each changed item takes the next anchor.
fn write_items(tx: &rusqlite::Transaction<'_>, items: &ItemChanges) -> Result<()> {
    let mut anchor: u64 = tx.query_row("SELECT anchor FROM drive", [], |row| {
        row.get::<_, i64>(0).map(|a| a as u64)
    })?;
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
    tx.execute("UPDATE drive SET anchor = ?1", [anchor as i64])?;
    Ok(())
}

fn writer_clock(conn: &Connection, writer: Writer) -> Result<DeviceClock> {
    let (device, last) = conn.query_row(
        "SELECT device, clock FROM writers WHERE writer = ?1",
        [writer.code()],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok(DeviceClock {
        device: DeviceId(array(device)?),
        last: last as u64,
    })
}

/// Never moves back: the clock is the device's last sequence number.
fn set_clock(tx: &rusqlite::Transaction<'_>, writer: Writer, clock: DeviceClock) -> Result<()> {
    tx.execute(
        "UPDATE writers SET clock = max(clock, ?2) WHERE writer = ?1",
        params![writer.code(), clock.last as i64],
    )?;
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
        let extension = reopened.clock(Writer::Extension).unwrap();
        assert_ne!(extension.device, clock.device);
        let folders = reopened.clock(Writer::Folders).unwrap();
        assert_ne!(folders.device, extension.device);
    }

    #[tokio::test]
    async fn the_descriptor_signed_in_with_is_kept() {
        let store = Store::memory().unwrap();
        let user = Keys::generate();
        let descriptor = crate::descriptor::Descriptor::generate("Lupine");
        let kept = DescriptorRecord {
            event: descriptor.seal(&user).await.unwrap(),
            name: descriptor.name.clone(),
            user_relays: vec![RelayUrl::parse("wss://user.example.com").unwrap()],
        };
        assert!(store.set_descriptor(&kept).is_err(), "no drive yet");
        store.set_drive(&record(1)).unwrap();
        assert_eq!(store.descriptor().unwrap(), None);
        store.set_descriptor(&kept).unwrap();
        assert_eq!(store.descriptor().unwrap(), Some(kept));
        assert_eq!(store.unsent().unwrap(), 0);
    }

    #[test]
    fn the_history_window_is_a_setting_of_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drive.sqlite");
        let store = Store::open(&path).unwrap();
        let default = Retention::default().history;
        assert_eq!(store.history().unwrap(), default);
        assert!(
            store.set_history(Duration::from_secs(60)).is_err(),
            "no drive yet"
        );

        store.set_drive(&record(1)).unwrap();
        assert_eq!(store.history().unwrap(), default);
        store.set_history(Duration::from_secs(7 * 86_400)).unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.history().unwrap(), Duration::from_secs(7 * 86_400));
    }

    #[test]
    fn version_1_databases_keep_their_device_and_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drive.sqlite");
        let device = DeviceId::random();
        let version = NodeVersion::folder(
            crate::node::Stamp::next(device, 0, 0),
            NodeId::random(),
            None,
            "Photos",
        );
        let id = VersionId([3; 32]);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            let drive = record(1);
            conn.execute(
                "INSERT INTO drive (singleton, id, pubkey, epoch, relays, servers, device, clock, synced, generation, anchor)
                 VALUES (1, ?1, ?2, 0, '[]', '[]', ?3, 42, 1, ?4, 0)",
                params![
                    drive.id.as_bytes(),
                    drive.pubkey.as_bytes(),
                    device.as_bytes(),
                    [9u8; 8]
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO versions (id, json) VALUES (?1, ?2)",
                params![id.as_bytes(), serde_json::to_string(&version).unwrap()],
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let (clock, synced) = store.device_state().unwrap();
        assert_eq!((clock.device, clock.last, synced), (device, 42, true));
        let extension = store.clock(Writer::Extension).unwrap();
        assert_ne!(extension.device, device);
        assert_eq!(extension.last, 0);
        let folders = store.clock(Writer::Folders).unwrap();
        assert_ne!(folders.device, extension.device);
        assert!(store.folders().unwrap().is_empty());
        let versions = store.node_versions(&version.node).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].0, id);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.clock(Writer::Extension).unwrap(), extension);
    }
}
