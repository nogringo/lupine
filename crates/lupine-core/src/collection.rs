//! Blob collection: deleting from the drive's servers the blobs no version
//! has referenced for [`GRACE`]: those of the versions compaction deleted,
//! and uploads whose version was never written.

use std::collections::HashSet;
use std::time::Duration;

use futures::{StreamExt, stream};

use crate::blob::BlobStore;
use crate::blossom::BlossomStore;
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;
use crate::node::unix_millis;
use crate::store::Store;

/// How long a blob stays once no version references it.
pub const GRACE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const READ_CONCURRENCY: usize = 8;
const DELETE_CONCURRENCY: usize = 8;

/// The blobs the versions this device holds reference: their chunks, their
/// chunk tables and the chunks the tables list.
#[derive(Debug, Default)]
pub(crate) struct References {
    pub blobs: HashSet<Sha256Hash>,
    /// Chunk tables that could not be read for now. What they list is
    /// missing from `blobs`.
    pub unread: Vec<(Sha256Hash, Error)>,
}

/// Reads each chunk table once: the local database keeps what it lists.
/// A table that no server serves intact, or that does not open, lists
/// nothing a reader could use.
pub(crate) async fn references(store: &Store, blobs: &impl BlobStore) -> Result<References> {
    let mut references = References::default();
    let mut tables = HashSet::new();
    let mut unread = Vec::new();
    for file in store.file_contents()? {
        let key = file.file_key();
        match file.content.table {
            Some(table) => {
                if !tables.insert(table) {
                    continue;
                }
                references.blobs.insert(table);
                match store.table_chunks(&table)? {
                    Some(chunks) => references.blobs.extend(chunks),
                    None => unread.push((table, file.content, key)),
                }
            }
            None => {
                // An inline table that does not decode lists nothing either.
                if let Ok(content) = file.content.resolve(blobs, &key).await {
                    references
                        .blobs
                        .extend(content.chunks().iter().map(|chunk| chunk.blob));
                }
            }
        }
    }
    let mut reads = stream::iter(unread)
        .map(|(table, content, key)| async move { (table, content.resolve(blobs, &key).await) })
        .buffer_unordered(READ_CONCURRENCY);
    while let Some((table, read)) = reads.next().await {
        match read {
            Ok(content) => {
                let chunks: Vec<_> = content.chunks().iter().map(|chunk| chunk.blob).collect();
                store.set_table_chunks(&table, &chunks)?;
                references.blobs.extend(chunks);
            }
            Err(
                e @ (Error::Http { .. } | Error::Network { .. } | Error::Store(_) | Error::Io(_)),
            ) => {
                references.unread.push((table, e));
            }
            Err(_) => {}
        }
    }
    Ok(references)
}

#[derive(Debug, Default)]
pub struct Collected {
    /// Blobs this device knows of that no version references.
    pub unreferenced: usize,
    /// Deleted from every drive server.
    pub deleted: usize,
    /// Left for the next collection.
    pub failed: Vec<(Sha256Hash, Error)>,
    /// Nothing was deleted: the drive holds a version this device cannot
    /// read, such as a later format, which may reference any blob.
    pub held_back: bool,
}

/// Deletes from every drive server the blobs that no version this device
/// holds has referenced for [`GRACE`], uploads that no version ever
/// referenced included. Run right after a full sync of every drive relay,
/// so that the device holds every version. The servers the drive leaves
/// keep their blobs. Fails, deleting nothing, while a chunk table cannot be
/// read.
pub async fn collect(store: &Store, keys: &DriveKeys) -> Result<Collected> {
    let Some(drive) = store.drive()? else {
        return Ok(Collected::default());
    };
    let leaving = store.lists_state()?.leaving;
    let blobs =
        BlossomStore::new(&drive.servers, keys.keys())?.with_sources(&leaving, keys.keys())?;
    collect_at(store, &blobs, unix_millis()).await
}

async fn collect_at(store: &Store, blobs: &impl BlobStore, now: i64) -> Result<Collected> {
    let held_back = store.holds_unreadable()?;
    let mut references = references(store, blobs).await?;
    if let Some((_, e)) = references.unread.pop() {
        return Err(e);
    }
    let unreferenced = store.mark_blobs(&references.blobs, now)?;
    let mut collected = Collected {
        unreferenced: unreferenced.len(),
        held_back,
        ..Collected::default()
    };
    if held_back {
        return Ok(collected);
    }
    let cutoff = now.saturating_sub(GRACE.as_millis() as i64);
    let due = unreferenced
        .into_iter()
        .filter(|(_, since)| *since <= cutoff)
        .map(|(hash, _)| hash);
    let mut deletions = stream::iter(due)
        .map(|hash| async move {
            // An upload may have noted it since.
            let deleted = match store.unreferenced_since(&hash, cutoff) {
                Ok(true) => blobs.delete(hash).await.map(|()| true),
                other => other,
            };
            (hash, deleted)
        })
        .buffer_unordered(DELETE_CONCURRENCY);
    while let Some((hash, deleted)) = deletions.next().await {
        match deleted {
            Ok(true) => {
                store.forget_blob(&hash, cutoff)?;
                collected.deleted += 1;
            }
            Ok(false) => {}
            Err(e) => collected.failed.push((hash, e)),
        }
    }
    Ok(collected)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use bytes::Bytes;
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;

    use super::*;
    use crate::blob::MemoryBlobStore;
    use crate::content::{INLINE_CHUNK_LIMIT, MIN_CHUNK_SIZE, TransferOptions};
    use crate::ids::DriveId;
    use crate::items::{Item, ItemChanges};
    use crate::replica::{Record, WrapState};
    use crate::session::DriveSession;
    use crate::store::DriveRecord;
    use crate::testing::TestRelay;
    use crate::writer::{Contents, Edit, ItemWriter, NewItem, Place};

    const CS: usize = MIN_CHUNK_SIZE as usize;
    const GRACE_MS: i64 = GRACE.as_millis() as i64;

    struct Setup {
        drive: DriveKeys,
        store: Arc<Store>,
        blobs: Arc<MemoryBlobStore>,
        writer: ItemWriter<Arc<MemoryBlobStore>>,
        dir: tempfile::TempDir,
    }

    impl Setup {
        fn new(relays: &[&TestRelay]) -> Self {
            let drive = DriveKeys::generate();
            let store = Store::memory().unwrap();
            store
                .set_drive(&DriveRecord {
                    id: DriveId([5; 16]),
                    pubkey: drive.public_key(),
                    epoch: 0,
                    relays: relays.iter().map(|r| r.url.clone()).collect(),
                    servers: Vec::new(),
                })
                .unwrap();
            let store = Arc::new(store);
            let blobs = Arc::new(MemoryBlobStore::new());
            let writer = ItemWriter::new(Arc::clone(&store), keys(&drive), Arc::clone(&blobs))
                .unwrap()
                .with_options(TransferOptions {
                    chunk_size: MIN_CHUNK_SIZE,
                    concurrency: 2,
                });
            Self {
                drive,
                store,
                blobs,
                writer,
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        async fn create(&self, name: &str, bytes: &[u8]) -> Item {
            let new = NewItem {
                request: name.into(),
                place: Place {
                    parent: None,
                    name: name.into(),
                },
                folder: false,
                may_exist: false,
                created: None,
                modified: None,
            };
            let path = self.path(name, bytes);
            self.writer
                .create(new, Some(Contents::File(&path)), &|_| {})
                .await
                .unwrap()
                .unwrap()
        }

        async fn rewrite(&self, item: &Item, bytes: &[u8]) -> Item {
            let edit = Edit {
                base_content: item.content_version(),
                ..Edit::default()
            };
            let path = self.path(&format!("{}.new", item.name), bytes);
            self.writer
                .modify(item.node, false, edit, Some(Contents::File(&path)), &|_| {})
                .await
                .unwrap()
        }

        async fn chunks(&self, item: &Item) -> Vec<Sha256Hash> {
            let file = item.file.as_ref().unwrap();
            let key = file.file_key();
            let content = file.content.resolve(&self.blobs, &key).await.unwrap();
            content.chunks().iter().map(|chunk| chunk.blob).collect()
        }

        async fn held(&self, hashes: &[Sha256Hash]) -> Vec<bool> {
            let mut held = Vec::new();
            for hash in hashes {
                held.push(self.blobs.get(*hash).await.is_ok());
            }
            held
        }
    }

    fn keys(drive: &DriveKeys) -> DriveKeys {
        DriveKeys::from_secret(drive.keys().secret_key().clone())
    }

    fn bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31) ^ seed)
            .collect()
    }

    #[tokio::test]
    async fn blobs_go_once_no_version_referenced_them_for_the_grace_period() {
        let relay = TestRelay::start().await;
        let s = Setup::new(&[&relay]);
        let edited = s.create("edited", &bytes(2 * CS, 1)).await;
        let old = s.chunks(&edited).await;
        let mut changed = bytes(2 * CS, 1);
        changed[CS] ^= 0xff;
        let edited = s.rewrite(&edited, &changed).await;
        let current = s.chunks(&edited).await;
        assert_eq!(old[0], current[0], "the first chunk did not change");

        let deleted = s.create("deleted", &bytes(CS, 2)).await;
        let gone = s.chunks(&deleted).await;
        s.writer
            .delete(deleted.node, deleted.content_version(), false)
            .unwrap();

        let orphan = Sha256Hash::digest(b"an upload whose version never came");
        s.blobs.put(orphan, Bytes::new()).await.unwrap();
        s.store.note_blobs(&[orphan], unix_millis()).unwrap();
        // A month after the uploads.
        let now = unix_millis() + GRACE_MS;

        let mut session = DriveSession::open(Arc::clone(&s.store), keys(&s.drive))
            .await
            .unwrap();
        s.store.set_history(Duration::ZERO).unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(session.sync().await.unwrap().failed.is_empty());
        assert_eq!(
            session.compact().await.unwrap(),
            2,
            "both superseded versions"
        );

        let first = collect_at(&s.store, &s.blobs, now).await.unwrap();
        assert_eq!((first.unreferenced, first.deleted), (3, 1), "{first:?}");
        assert_eq!(
            s.held(&[orphan, old[1], gone[0]]).await,
            [false, true, true]
        );

        let later = collect_at(&s.store, &s.blobs, now + GRACE_MS)
            .await
            .unwrap();
        assert_eq!((later.unreferenced, later.deleted), (2, 2), "{later:?}");
        assert_eq!(s.held(&[old[1], gone[0]]).await, [false, false]);
        assert_eq!(s.held(&current).await, [true, true]);

        let after = collect_at(&s.store, &s.blobs, now + 2 * GRACE_MS)
            .await
            .unwrap();
        assert_eq!((after.unreferenced, after.deleted), (0, 0), "forgotten");
    }

    #[tokio::test]
    async fn a_version_this_device_cannot_read_holds_every_blob_back() {
        let s = Setup::new(&[]);
        let orphan = Sha256Hash::digest(b"orphan");
        s.blobs.put(orphan, Bytes::new()).await.unwrap();
        let now = unix_millis();
        s.store.note_blobs(&[orphan], now - GRACE_MS).unwrap();

        let later = EventBuilder::new(Kind::GiftWrap, "a later format")
            .tag(Tag::public_key(s.drive.public_key()))
            .finalize(&Keys::generate())
            .unwrap();
        let record = Record::Wrap {
            id: later.id,
            created_at: later.created_at,
            state: WrapState::Unreadable,
            event: Some(later),
        };
        let (clock, _) = s.store.device_state().unwrap();
        s.store
            .commit(&[record], clock, true, &ItemChanges::default())
            .unwrap();

        let collected = collect_at(&s.store, &s.blobs, now).await.unwrap();
        assert!(collected.held_back);
        assert_eq!((collected.unreferenced, collected.deleted), (1, 0));
        assert_eq!(s.held(&[orphan]).await, [true]);
    }

    /// Fails every read, as an unreachable server does.
    struct Unreachable;

    impl BlobStore for Unreachable {
        async fn put(&self, _: Sha256Hash, _: Bytes) -> Result<()> {
            unreachable!()
        }

        async fn get(&self, _: Sha256Hash) -> Result<Bytes> {
            Err(Error::Network {
                server: "test".into(),
                message: "down".into(),
            })
        }

        async fn delete(&self, _: Sha256Hash) -> Result<()> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn chunk_tables_are_read_once_and_must_be_read() {
        let s = Setup::new(&[]);
        let large = s
            .create("large", &bytes((INLINE_CHUNK_LIMIT + 1) * CS, 3))
            .await;
        let table = large.file.as_ref().unwrap().content.table.unwrap();
        let chunks = s.chunks(&large).await;
        let now = unix_millis();

        assert!(
            collect_at(&s.store, &Unreachable, now).await.is_ok(),
            "the upload kept what the table lists"
        );
        s.store.lock().execute("DELETE FROM tables", []).unwrap();
        let unread = collect_at(&s.store, &Unreachable, now).await;
        assert!(matches!(unread, Err(Error::Network { .. })), "{unread:?}");

        let read = collect_at(&s.store, &s.blobs, now).await.unwrap();
        assert_eq!(read.unreferenced, 0);
        assert_eq!(s.store.table_chunks(&table).unwrap().unwrap(), chunks);

        s.blobs.delete(table).await.unwrap();
        let later = collect_at(&s.store, &s.blobs, now + GRACE_MS)
            .await
            .unwrap();
        assert_eq!((later.unreferenced, later.deleted), (0, 0), "{later:?}");
    }
}
