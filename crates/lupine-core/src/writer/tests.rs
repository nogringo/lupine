use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::blob::MemoryBlobStore;
use crate::content::MIN_CHUNK_SIZE;
use crate::ids::{DeviceId, DriveId};
use crate::io::MemoryFile;
use crate::node::Stamp;
use crate::store::DriveRecord;

const CS: usize = MIN_CHUNK_SIZE as usize;

struct Setup {
    keys: DriveKeys,
    writer: ItemWriter<MemoryBlobStore>,
    dir: TempDir,
    files: usize,
}

impl Setup {
    fn new() -> Self {
        let keys = DriveKeys::generate();
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: DriveId([7; 16]),
                pubkey: keys.public_key(),
                epoch: 0,
                relays: Vec::new(),
                servers: Vec::new(),
            })
            .unwrap();
        let same = DriveKeys::from_secret(keys.keys().secret_key().clone());
        let writer = ItemWriter::new(Arc::new(store), same, MemoryBlobStore::new())
            .unwrap()
            .with_options(TransferOptions {
                chunk_size: MIN_CHUNK_SIZE,
                concurrency: 2,
            });
        Self {
            keys,
            writer,
            dir: tempfile::tempdir().unwrap(),
            files: 0,
        }
    }

    fn store(&self) -> &Store {
        self.writer.store()
    }

    fn file(&mut self, bytes: &[u8]) -> PathBuf {
        self.files += 1;
        let path = self.dir.path().join(format!("{}", self.files));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    async fn folder(&self, parent: Option<NodeId>, name: &str) -> Item {
        let new = NewItem {
            request: format!("folder {name}"),
            place: Place {
                parent,
                name: name.into(),
            },
            folder: true,
            may_exist: false,
            created: None,
            modified: None,
        };
        self.writer
            .create(new, None, &|_| {})
            .await
            .unwrap()
            .unwrap()
    }

    async fn create(&mut self, parent: Option<NodeId>, name: &str, bytes: &[u8]) -> Item {
        let path = self.file(bytes);
        let new = NewItem {
            request: format!("file {name}"),
            place: Place {
                parent,
                name: name.into(),
            },
            folder: false,
            may_exist: false,
            created: Some(1),
            modified: Some(2),
        };
        self.writer
            .create(new, Some(&path), &|_| {})
            .await
            .unwrap()
            .unwrap()
    }

    async fn rewrite(&mut self, item: &Item, bytes: &[u8]) -> Item {
        let path = self.file(bytes);
        let edit = Edit {
            base_content: item.content_version(),
            ..Edit::default()
        };
        self.writer
            .modify(item.node, false, edit, Some(&path), &|_| {})
            .await
            .unwrap()
    }

    async fn read(&self, item: &Item) -> Vec<u8> {
        let file = item.file.as_ref().unwrap();
        let (blobs, key) = (self.writer.blobs(), file.file_key());
        let content = file.content.resolve(blobs, &key).await.unwrap();
        let sink = Arc::new(MemoryFile::default());
        let options = TransferOptions::default();
        let all = 0..file.content.size;
        content::download(
            blobs,
            &content,
            &key,
            all,
            Arc::clone(&sink),
            options,
            &|_| {},
        )
        .await
        .unwrap();
        sink.snapshot()
    }

    fn heads(&self, node: &NodeId) -> Vec<NodeVersion> {
        let versions = self.store().node_versions(node).unwrap();
        let heads = history(&versions).heads().unwrap();
        heads.iter().map(|(_, v)| v.clone()).collect()
    }

    fn outbox(&self) -> Vec<bool> {
        self.store()
            .outbox(0)
            .unwrap()
            .iter()
            .map(|o| o.barrier)
            .collect()
    }

    /// Another device's version, as the session stores what it receives.
    fn remote(&self, node: &NodeId, change: impl FnOnce(&mut NodeVersion)) {
        let versions = self.store().node_versions(node).unwrap();
        let heads = history(&versions).heads().unwrap();
        let mut next = heads.winner().1.clone();
        change(&mut next);
        let stamp = Stamp::next(DeviceId::random(), 0, 0);
        let mut version = next.rewritten(stamp, &heads.versions()).unwrap();
        // Before what this device writes next, whatever the clock's resolution.
        version.time -= 1000;
        let (id, event) = self.keys.wrap(&version).unwrap();
        let mut changes = ItemChanges::default();
        if version.is_tombstone() {
            changes.removed.push(*node);
        } else {
            let current = self.store().item(node).unwrap().unwrap();
            changes
                .updated
                .push(shown(&version, current.parent, version.name.clone()));
        }
        let records = [
            Record::Wrap {
                id: event.id,
                created_at: event.created_at,
                state: WrapState::Version(id),
                event: Some(event),
            },
            Record::Version { id, version },
        ];
        let (clock, _) = self.store().device_state().unwrap();
        self.store()
            .commit(&records, clock, false, &changes)
            .unwrap();
    }
}

#[tokio::test]
async fn creations_write_versions_and_the_items_they_show() {
    let mut s = Setup::new();
    let photos = s.folder(None, "Photos").await;
    let bytes: Vec<u8> = (0..3 * CS + 10).map(|i| (i % 251) as u8).collect();
    let beach = s.create(Some(photos.node), "beach.jpg", &bytes).await;

    assert_eq!(s.store().item(&photos.node).unwrap(), Some(photos.clone()));
    let stored = s.store().item(&beach.node).unwrap().unwrap();
    assert_eq!(stored, beach);
    assert_eq!(
        (
            stored.parent,
            stored.size(),
            stored.created,
            stored.modified
        ),
        (Some(photos.node), bytes.len() as u64, Some(1), Some(2))
    );
    assert_eq!(s.read(&beach).await, bytes);

    let heads = s.heads(&beach.node);
    let extension = s.store().clock(Writer::Extension).unwrap();
    let (session, _) = s.store().device_state().unwrap();
    assert_eq!(heads[0].device, extension.device);
    assert_ne!(extension.device, session.device, "a device id of its own");
    assert_eq!(extension.last, heads[0].seq());
    assert_eq!(s.outbox(), [false, false]);
}

#[tokio::test]
async fn a_replayed_creation_finds_its_item() {
    let mut s = Setup::new();
    let first = s.create(None, "notes.txt", b"one").await;
    let replayed = s.create(None, "notes.txt", b"one").await;
    assert_eq!(replayed, first);
    assert_eq!(s.outbox().len(), 1);
}

#[tokio::test]
async fn imports_match_items_of_the_same_name() {
    let mut s = Setup::new();
    let notes = s.create(None, "notes.txt", b"hello").await;
    let import = |name: &str| NewItem {
        request: "imported again".into(),
        place: Place {
            parent: None,
            name: name.into(),
        },
        folder: false,
        may_exist: true,
        created: None,
        modified: None,
    };
    let matched = s.writer.create(import("notes.txt"), None, &|_| {}).await;
    assert_eq!(matched.unwrap(), Some(notes));
    let unmatched = s.writer.create(import("other.txt"), None, &|_| {}).await;
    assert_eq!(unmatched.unwrap(), None);
    assert_eq!(s.outbox().len(), 1, "nothing written");
}

#[tokio::test]
async fn moves_and_renames_replace_the_version_shown() {
    let mut s = Setup::new();
    let archive = s.folder(None, "Archive").await;
    let report = s.create(None, "report.pdf", b"pdf").await;
    let edit = Edit {
        base_content: report.content_version(),
        place: Some(Place {
            parent: Some(archive.node),
            name: "cafe\u{301}.pdf".into(),
        }),
        ..Edit::default()
    };
    let moved = s
        .writer
        .modify(report.node, false, edit.clone(), None, &|_| {})
        .await
        .unwrap();
    assert_eq!(
        (moved.parent, moved.name.as_str()),
        (Some(archive.node), "caf\u{e9}.pdf")
    );
    assert_eq!(moved.content_version(), report.content_version());
    assert_eq!(s.store().item(&report.node).unwrap(), Some(moved.clone()));
    assert_eq!(s.heads(&report.node).len(), 1);

    let again = s
        .writer
        .modify(report.node, false, edit, None, &|_| {})
        .await
        .unwrap();
    assert_eq!(again, moved);
    assert_eq!(
        s.outbox().len(),
        3,
        "an edit that changes nothing writes nothing"
    );

    let nowhere = Edit {
        place: Some(Place {
            parent: Some(NodeId::random()),
            name: "x".into(),
        }),
        ..Edit::default()
    };
    let refused = s.writer.modify(report.node, false, nowhere, None, &|_| {});
    assert!(matches!(refused.await, Err(Error::UnknownParent)));
}

#[tokio::test]
async fn new_content_uploads_only_the_chunks_that_changed() {
    let mut s = Setup::new();
    let mut bytes: Vec<u8> = (0..4 * CS).map(|i| (i % 251) as u8).collect();
    let file = s.create(None, "video.mov", &bytes).await;
    let puts = s.writer.blobs().put_count();
    assert_eq!(puts, 4);

    bytes[CS + 7] ^= 0xff;
    let edited = s.rewrite(&file, &bytes).await;
    assert_eq!(s.writer.blobs().put_count(), puts + 1);
    assert_ne!(edited.content_version(), file.content_version());
    let (old, new) = (file.file.unwrap(), edited.file.clone().unwrap());
    assert_eq!(old.key.as_bytes(), new.key.as_bytes(), "same key");
    assert_eq!(s.read(&edited).await, bytes);
    assert_eq!(s.heads(&edited.node).len(), 1);
}

#[tokio::test]
async fn content_edited_from_an_older_version_stays_in_conflict() {
    let mut s = Setup::new();
    let notes = s.create(None, "notes.txt", b"draft").await;
    let elsewhere = FileMeta {
        key: FileKey::generate().root().clone(),
        ..notes.file.clone().unwrap()
    };
    s.remote(&notes.node, |v| v.file = Some(elsewhere.clone()));

    let mine = s.rewrite(&notes, b"mine").await;
    let heads = s.heads(&notes.node);
    assert_eq!(heads.len(), 2, "the edit replaced nothing it did not see");
    assert!(heads.iter().any(|h| h.file.as_ref() == Some(&elsewhere)));
    assert_eq!(s.store().item(&notes.node).unwrap(), Some(mine.clone()));

    let versions = s.store().node_versions(&notes.node).unwrap();
    let history = history(&versions);
    let heads = history.heads().unwrap();
    assert_eq!(heads.winner().1.file, mine.file, "the later edit wins");
    assert!(heads.in_conflict());
}

#[tokio::test]
async fn an_edit_brings_back_a_node_deleted_elsewhere() {
    let mut s = Setup::new();
    let notes = s.create(None, "notes.txt", b"keep me").await;
    s.remote(&notes.node, |v| {
        v.node_type = NodeType::Deleted;
        v.file = None;
    });
    assert!(s.store().item(&notes.node).unwrap().is_none());

    let edit = Edit {
        base_content: notes.content_version(),
        place: Some(Place {
            parent: None,
            name: "kept.txt".into(),
        }),
        ..Edit::default()
    };
    let back = s
        .writer
        .modify(notes.node, false, edit, None, &|_| {})
        .await
        .unwrap();
    assert_eq!(back.name, "kept.txt");
    assert_eq!(back.file, notes.file);
    let heads = s.heads(&notes.node);
    assert_eq!(heads.len(), 1);
    assert!(!heads[0].is_tombstone());
}

#[tokio::test]
async fn deleting_a_folder_tombstones_its_contents_deepest_first() {
    let mut s = Setup::new();
    let photos = s.folder(None, "Photos").await;
    let trip = s.folder(Some(photos.node), "Trip").await;
    let beach = s.create(Some(trip.node), "beach.jpg", b"jpg").await;
    let sunset = s.create(Some(trip.node), "sunset.jpg", b"jpg").await;
    let before = s.outbox().len();

    let refused = s.writer.delete(photos.node, None, false);
    assert!(matches!(refused, Err(Error::FolderNotEmpty)));
    assert_eq!(
        s.writer.delete(photos.node, None, true).unwrap(),
        Deletion::Deleted(4)
    );
    for item in [&photos, &trip, &beach, &sunset] {
        assert!(s.store().item(&item.node).unwrap().is_none());
        assert!(s.heads(&item.node)[0].is_tombstone());
    }
    assert_eq!(
        s.outbox()[before..],
        [true, false, true, true],
        "one wave per level"
    );
    assert_eq!(
        s.writer.delete(photos.node, None, true).unwrap(),
        Deletion::Unknown
    );
}

#[tokio::test]
async fn a_file_edited_since_is_kept_instead_of_deleted() {
    let mut s = Setup::new();
    let notes = s.create(None, "notes.txt", b"draft").await;
    let anchor = s.store().anchor().unwrap();
    let elsewhere = FileMeta {
        key: FileKey::generate().root().clone(),
        ..notes.file.clone().unwrap()
    };
    s.remote(&notes.node, |v| v.file = Some(elsewhere));
    let seen = s.store().changes(&anchor.to_bytes(), 10).unwrap().anchor;

    let kept = s.writer.delete(notes.node, notes.content_version(), true);
    let Deletion::Kept(item) = kept.unwrap() else {
        panic!("deleted content the integration never showed");
    };
    assert_eq!(s.store().item(&notes.node).unwrap(), Some(item.clone()));
    let changes = s.store().changes(&seen.to_bytes(), 10).unwrap();
    assert_eq!(changes.updated, [item], "announced again");
}

#[tokio::test]
async fn names_are_checked() {
    let s = Setup::new();
    let new = NewItem {
        request: "bad".into(),
        place: Place {
            parent: None,
            name: "a/b".into(),
        },
        folder: true,
        may_exist: false,
        created: None,
        modified: None,
    };
    let refused = s.writer.create(new, None, &|_| {}).await;
    assert!(matches!(refused, Err(Error::InvalidName)));
    let orphan = NewItem {
        request: "orphan".into(),
        place: Place {
            parent: Some(NodeId::random()),
            name: "a".into(),
        },
        folder: true,
        may_exist: false,
        created: None,
        modified: None,
    };
    let refused = s.writer.create(orphan, None, &|_| {}).await;
    assert!(matches!(refused, Err(Error::UnknownParent)));
}
