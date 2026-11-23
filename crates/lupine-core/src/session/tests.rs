use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use nostr::event::{Event, EventBuilder, EventId, FinalizeEvent, Kind};
use nostr::filter::Filter;
use nostr::key::{Keys, PublicKey};
use nostr::nips::nip17::PrivateDirectMessageBuilder;
use nostr::types::{RelayUrl, Timestamp};
use nostr_database::error::Error as DatabaseError;
use nostr_database::{
    DatabaseEventStatus, Features, NostrDatabase, RejectedReason, SaveEventStatus,
};
use nostr_memory::MemoryDatabase;
use nostr_sdk::local_relay::{LocalRelay, LocalRelayBuilder};

use super::*;
use crate::account::Account;
use crate::blob::MemoryBlobStore;
use crate::content::{ContentDescriptor, FileKey};
use crate::ids::DriveId;
use crate::store::{DriveRecord, Store};
use crate::writer::{ItemWriter, NewItem, Place};

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T, DatabaseError>> + Send + 'a>>;

/// A relay store that applies NIP-59: a deletion from a gift wrap's
/// recipient deletes it. nostr-memory 0.45 applies only NIP-09's author
/// rule, and refuses such deletions outright.
#[derive(Debug)]
struct Nip59Store {
    inner: MemoryDatabase,
    deleted: Mutex<HashSet<EventId>>,
    /// Named by a deletion before they arrived, with who deleted them.
    pending: Mutex<HashMap<EventId, PublicKey>>,
}

impl Nip59Store {
    fn new() -> Self {
        Self {
            inner: MemoryDatabase::builder().process_nip09(false).build(),
            deleted: Mutex::default(),
            pending: Mutex::default(),
        }
    }

    fn may_delete(target: &Event, deleter: &PublicKey) -> bool {
        target.pubkey == *deleter
            || (target.kind == Kind::GiftWrap && target.tags.public_keys().any(|pk| pk == *deleter))
    }
}

impl NostrDatabase for Nip59Store {
    fn backend(&self) -> &'static str {
        "nip59-test"
    }

    fn features(&self) -> Features {
        self.inner.features()
    }

    fn save_event<'a>(&'a self, event: &'a Event) -> Boxed<'a, SaveEventStatus> {
        Box::pin(async move {
            let deleter = self.pending.lock().unwrap().get(&event.id).copied();
            let refused = self.deleted.lock().unwrap().contains(&event.id)
                || deleter.is_some_and(|d| Self::may_delete(event, &d));
            if refused {
                return Ok(SaveEventStatus::Rejected(RejectedReason::Deleted));
            }
            if event.kind == Kind::EventDeletion {
                for id in event.tags.event_ids() {
                    match self.inner.event_by_id(&id).await? {
                        Some(target) if Self::may_delete(&target, &event.pubkey) => {
                            self.inner.delete(Filter::new().id(id)).await?;
                            self.deleted.lock().unwrap().insert(id);
                        }
                        Some(_) => {}
                        None => {
                            self.pending.lock().unwrap().insert(id, event.pubkey);
                        }
                    }
                }
            }
            self.inner.save_event(event).await
        })
    }

    fn check_id<'a>(&'a self, id: &'a EventId) -> Boxed<'a, DatabaseEventStatus> {
        Box::pin(async move {
            if self.deleted.lock().unwrap().contains(id) {
                return Ok(DatabaseEventStatus::Deleted);
            }
            self.inner.check_id(id).await
        })
    }

    fn event_by_id<'a>(&'a self, id: &'a EventId) -> Boxed<'a, Option<Event>> {
        self.inner.event_by_id(id)
    }

    fn count(&self, filter: Filter) -> Boxed<'_, usize> {
        self.inner.count(filter)
    }

    fn query(&self, filter: Filter) -> Boxed<'_, BTreeSet<Event>> {
        self.inner.query(filter)
    }

    fn negentropy_items(&self, filter: Filter) -> Boxed<'_, Vec<(EventId, Timestamp)>> {
        self.inner.negentropy_items(filter)
    }

    fn delete(&self, filter: Filter) -> Boxed<'_, ()> {
        self.inner.delete(filter)
    }

    fn wipe(&self) -> Boxed<'_, ()> {
        self.inner.wipe()
    }
}

/// Serves gift wraps only to their recipient, after NIP-42 AUTH, as NIP-17
/// asks.
struct TestRelay {
    relay: LocalRelay,
    store: Arc<Nip59Store>,
    url: RelayUrl,
}

impl TestRelay {
    async fn start() -> Self {
        Self::with(|builder| builder).await
    }

    async fn with(configure: impl FnOnce(LocalRelayBuilder) -> LocalRelayBuilder) -> Self {
        let store = Arc::new(Nip59Store::new());
        let builder = LocalRelay::builder()
            .database(Arc::clone(&store))
            .auth_dm(true);
        let relay = configure(builder).build();
        relay.run().await.unwrap();
        let url = relay.url().await;
        Self { relay, store, url }
    }

    async fn holds(&self, id: EventId) -> bool {
        self.store.event_by_id(&id).await.unwrap().is_some()
    }

    async fn wraps(&self) -> usize {
        let filter = Filter::new().kind(Kind::GiftWrap);
        self.store.count(filter).await.unwrap()
    }
}

fn store(drive: &DriveKeys, relays: &[&TestRelay]) -> Arc<Store> {
    let store = Store::memory().unwrap();
    store
        .set_drive(&DriveRecord {
            id: DriveId([7; 16]),
            pubkey: drive.public_key(),
            epoch: 0,
            relays: relays.iter().map(|r| r.url.clone()).collect(),
            servers: Vec::new(),
        })
        .unwrap();
    Arc::new(store)
}

async fn open(drive: &DriveKeys, store: &Arc<Store>) -> DriveSession {
    let keys = DriveKeys::from_secret(drive.keys().secret_key().clone());
    DriveSession::open(Arc::clone(store), keys).await.unwrap()
}

async fn device(drive: &DriveKeys, relays: &[&TestRelay]) -> DriveSession {
    open(drive, &store(drive, relays)).await
}

fn content(byte: u8) -> ContentDescriptor {
    ContentDescriptor {
        size: 1,
        chunk_size: 1 << 16,
        chunks: Some(vec![byte; 64]),
        table: None,
    }
}

fn listing(session: &DriveSession) -> Vec<String> {
    let tree = session.tree();
    let mut paths: Vec<String> = tree
        .entries()
        .map(|e| tree.path(&e.node).unwrap().join("/"))
        .collect();
    paths.sort();
    paths
}

async fn synced(session: &mut DriveSession) -> SyncReport {
    let report = session.sync().await.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    report
}

async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

#[tokio::test]
async fn a_second_device_sees_what_the_first_wrote() {
    let (a, b) = (TestRelay::start().await, TestRelay::start().await);
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a, &b]).await;
    let (photos, published) = first.create_folder(None, "Photos").await.unwrap();
    assert_eq!(
        published.sent.accepted.len(),
        2,
        "{}",
        published.sent.reasons()
    );
    first
        .create_file(
            Some(photos),
            "beach.jpg",
            &FileKey::generate(),
            content(1),
            None,
        )
        .await
        .unwrap();

    let mut second = device(&drive, &[&a, &b]).await;
    assert!(!second.is_complete());
    let report = synced(&mut second).await;
    assert_eq!((report.received, report.repaired), (2, 0));
    assert_eq!(report.changed.len(), 2);
    assert!(second.is_complete());
    assert_eq!(listing(&second), ["Photos", "Photos/beach.jpg"]);

    assert_eq!(synced(&mut second).await.received, 0);
}

#[tokio::test]
async fn relays_that_missed_wraps_get_them_back() {
    let (a, b) = (TestRelay::start().await, TestRelay::start().await);
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a]).await;
    first.create_folder(None, "Written to A").await.unwrap();
    assert_eq!(b.wraps().await, 0);

    let mut second = device(&drive, &[&a, &b]).await;
    assert_eq!(synced(&mut second).await.repaired, 1);
    assert_eq!(b.wraps().await, 1);

    let mut third = device(&drive, &[&b]).await;
    synced(&mut third).await;
    assert_eq!(listing(&third), ["Written to A"]);
}

#[tokio::test]
async fn compaction_deletes_superseded_wraps_as_their_recipient() {
    let (a, b) = (TestRelay::start().await, TestRelay::start().await);
    let drive = DriveKeys::generate();
    let retention = Retention {
        history: Duration::ZERO,
        ..Retention::default()
    };
    let mut first = device(&drive, &[&a]).await.with_retention(retention);
    let (node, draft) = first.create_folder(None, "Draft").await.unwrap();
    synced(&mut device(&drive, &[&a, &b]).await).await;
    assert!(b.holds(draft.wrap).await);

    tick().await;
    first.move_to(node, None, "Final").await.unwrap();
    synced(&mut first).await;
    tick().await;
    assert_eq!(first.compact().await.unwrap(), 1);
    assert!(
        !a.holds(draft.wrap).await,
        "A honors the recipient's deletion"
    );
    assert!(b.holds(draft.wrap).await, "B was not reached");

    let mut second = device(&drive, &[&a, &b]).await;
    synced(&mut second).await;
    assert!(!b.holds(draft.wrap).await, "the deletion reached B");
    assert_eq!(b.wraps().await, 1, "with the wrap B lacked");
    assert_eq!(listing(&second), ["Final"]);
    assert!(second.replica().is_deleted(&draft.wrap));
}

#[tokio::test]
async fn junk_sent_to_the_drive_is_deleted() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let spam = PrivateDirectMessageBuilder::new(drive.public_key(), "hello")
        .finalize(&Keys::generate())
        .unwrap();
    a.relay.add_event(spam.clone()).await.unwrap();

    let mut session = device(&drive, &[&a]).await;
    assert_eq!(synced(&mut session).await.received, 1);
    assert!(session.tree().is_empty());
    assert_eq!(session.compact().await.unwrap(), 1);
    assert!(!a.holds(spam.id).await);
}

#[tokio::test]
async fn concurrent_edits_settle_into_a_winner_and_a_copy() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a]).await;
    let (node, _) = first
        .create_file(None, "notes.txt", &FileKey::generate(), content(1), None)
        .await
        .unwrap();
    let mut second = device(&drive, &[&a]).await;
    synced(&mut second).await;

    let key = FileKey::generate();
    first
        .replace_content(node, &key, content(2), None)
        .await
        .unwrap();
    tick().await;
    second
        .replace_content(node, &key, content(3), None)
        .await
        .unwrap();

    synced(&mut first).await;
    assert_eq!(first.replica().heads(&node).unwrap().len(), 2);
    assert_eq!(listing(&first), ["notes.txt"]);
    assert_eq!(first.settle().await.unwrap(), 2, "a resolution and a copy");

    synced(&mut second).await;
    assert_eq!(second.settle().await.unwrap(), 0, "already settled");
    let shown = listing(&second);
    assert_eq!(shown, listing(&first));
    assert_eq!(shown.len(), 2);
    assert!(shown[0].starts_with("notes (conflict ") && shown[0].ends_with(").txt"));

    let heads = second.replica().heads(&node).unwrap();
    assert_eq!(heads.len(), 1);
    let file = heads.winner().1.file.as_ref().unwrap();
    assert_eq!(
        file.content.chunks,
        Some(vec![3; 64]),
        "the later edit wins"
    );
}

#[tokio::test]
async fn colliding_names_are_repaired_once() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a]).await;
    let mut second = device(&drive, &[&a]).await;
    first.create_folder(None, "Report").await.unwrap();
    second.create_folder(None, "report").await.unwrap();
    synced(&mut first).await;
    synced(&mut second).await;
    assert_eq!(listing(&first), listing(&second));
    assert!(listing(&first).iter().any(|name| name.ends_with(" (2)")));

    assert_eq!(first.settle().await.unwrap(), 1);
    synced(&mut second).await;
    assert_eq!(second.settle().await.unwrap(), 0);
    assert_eq!(listing(&first), listing(&second));
    assert!(second.tree().repairs().is_empty());
}

#[tokio::test]
async fn deleting_a_folder_deletes_what_it_holds() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a]).await;
    let (photos, _) = first.create_folder(None, "Photos").await.unwrap();
    let (trip, _) = first.create_folder(Some(photos), "Trip").await.unwrap();
    first
        .create_file(
            Some(trip),
            "beach.jpg",
            &FileKey::generate(),
            content(1),
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.delete(photos).await.unwrap(), 3);
    assert!(first.tree().is_empty());

    let mut second = device(&drive, &[&a]).await;
    synced(&mut second).await;
    assert!(second.tree().is_empty());
    assert_eq!(second.replica().node_ids().count(), 3, "tombstones stay");
    assert_eq!(second.settle().await.unwrap(), 0);
}

#[tokio::test]
async fn a_move_into_a_deleted_folder_brings_it_back() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut first = device(&drive, &[&a]).await;
    let (archive, _) = first.create_folder(None, "Archive").await.unwrap();
    let (report, _) = first
        .create_file(None, "report.pdf", &FileKey::generate(), content(1), None)
        .await
        .unwrap();
    let mut second = device(&drive, &[&a]).await;
    synced(&mut second).await;

    first.delete(archive).await.unwrap();
    second
        .move_to(report, Some(archive), "report.pdf")
        .await
        .unwrap();

    synced(&mut first).await;
    assert_eq!(listing(&first), ["Archive", "Archive/report.pdf"]);
    assert!(first.tree().get(&archive).unwrap().revived);
    assert_eq!(first.settle().await.unwrap(), 1);
    assert!(!first.tree().get(&archive).unwrap().revived);
}

#[tokio::test]
async fn new_versions_arrive_live() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut watcher = device(&drive, &[&a]).await;
    synced(&mut watcher).await;
    let mut events = Box::pin(watcher.watch().await.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut writer = device(&drive, &[&a]).await;
    let (node, _) = writer.create_folder(None, "Live").await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("the wrap arrives")
        .unwrap();
    assert_eq!(watcher.receive(&[event]).unwrap(), BTreeSet::from([node]));
    assert_eq!(listing(&watcher), ["Live"]);
}

#[tokio::test]
async fn a_new_device_finds_the_drive_on_the_users_relays() {
    let (user_relay, drive_relay) = (TestRelay::start().await, TestRelay::start().await);
    let user = Arc::new(Keys::generate());
    let account = Account::connect(Arc::clone(&user), [user_relay.url.clone()])
        .await
        .unwrap();
    let servers = ["https://blossom.example.com".to_owned()];
    let (created, lists) = account
        .create_drive("Lupine", std::slice::from_ref(&drive_relay.url), &servers)
        .await
        .unwrap();
    assert!(drive_relay.holds(created.event.id).await);

    let elsewhere = Account::connect(Arc::clone(&user), [user_relay.url.clone()])
        .await
        .unwrap();
    let drives = elsewhere.drives().await.unwrap();
    assert_eq!(drives.len(), 1);
    let found = &drives[0];
    assert_eq!(found.descriptor.id, created.descriptor.id);
    let drive_key = found.descriptor.keys().public_key();
    assert_eq!(elsewhere.lists(drive_key).await.unwrap(), lists);

    let fresh = TestRelay::start().await;
    let both = Account::connect(user, [user_relay.url.clone(), fresh.url.clone()])
        .await
        .unwrap();
    let sent = both.keep(&found.event).await;
    assert_eq!(sent.accepted, std::slice::from_ref(&fresh.url));
    assert!(fresh.holds(found.event.id).await);
}

#[tokio::test]
async fn listings_page_through_small_relay_limits() {
    let relay = TestRelay::with(|builder| builder.max_filter_limit(2)).await;
    let author = Keys::generate();
    for secs in 1..=5 {
        let note = EventBuilder::new(Kind::TextNote, format!("note {secs}"))
            .custom_created_at(Timestamp::from_secs(secs))
            .finalize(&author)
            .unwrap();
        relay.relay.add_event(note).await.unwrap();
    }
    let relays = Relays::for_user(Arc::new(Keys::generate()), [relay.url.clone()])
        .await
        .unwrap();
    let notes = relays
        .query(&Filter::new().kind(Kind::TextNote))
        .await
        .unwrap();
    assert_eq!(notes.len(), 5);
}

#[tokio::test]
async fn relays_serve_the_wraps_only_to_the_drive() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut owner = device(&drive, &[&a]).await;
    owner.create_folder(None, "Private").await.unwrap();

    let stranger = Relays::for_drive(&DriveKeys::generate(), [a.url.clone()])
        .await
        .unwrap();
    let wraps = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(drive.public_key());
    let compared = stranger.compare(&a.url, &wraps, &[]).await;
    assert!(compared.is_err(), "{compared:?}");
    assert_eq!(synced(&mut owner).await.received, 0);
    assert_eq!(listing(&owner), ["Private"]);
}

#[tokio::test]
async fn a_device_reopens_its_drive_from_the_local_database() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut writer = device(&drive, &[&a]).await;
    let (photos, _) = writer.create_folder(None, "Photos").await.unwrap();
    let (draft, _) = writer.create_folder(Some(photos), "Draft").await.unwrap();
    writer.delete(draft).await.unwrap();

    let local = store(&drive, &[&a]);
    let mut reader = open(&drive, &local).await;
    synced(&mut reader).await;
    let before = listing(&reader);
    let clock = reader.clock();
    reader.shutdown().await;

    let mut reopened = open(&drive, &local).await;
    assert_eq!(listing(&reopened), before, "without syncing");
    assert_eq!(reopened.clock().device, clock.device);
    assert_eq!(reopened.replica().node_ids().count(), 2, "tombstones too");
    let report = synced(&mut reopened).await;
    assert_eq!((report.received, report.repaired), (0, 0));
}

#[tokio::test]
async fn the_change_log_follows_the_tree() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut writer = device(&drive, &[&a]).await;
    let (photos, _) = writer.create_folder(None, "Photos").await.unwrap();
    let (beach, _) = writer
        .create_file(
            Some(photos),
            "beach.jpg",
            &FileKey::generate(),
            content(1),
            Some(5),
        )
        .await
        .unwrap();

    let local = store(&drive, &[&a]);
    let empty = local.anchor().unwrap();
    let mut reader = open(&drive, &local).await;
    synced(&mut reader).await;

    let changes = local.changes(&empty.to_bytes(), 100).unwrap();
    let updated: Vec<_> = changes.updated.iter().map(|i| i.node).collect();
    assert_eq!(updated, [photos, beach], "parents first");
    assert!(!changes.more);
    let item = local.item(&beach).unwrap().unwrap();
    assert_eq!(
        (item.parent, item.size(), item.modified),
        (Some(photos), 1, Some(5))
    );
    assert_eq!(local.children(None, None, 10).unwrap().len(), 1);
    assert_eq!(
        local.children(Some(photos), None, 10).unwrap()[0].node,
        beach
    );

    let first = local.changes(&empty.to_bytes(), 1).unwrap();
    assert!(first.more);
    let rest = local.changes(&first.anchor.to_bytes(), 1).unwrap();
    assert_eq!(rest.updated[0].node, beach);
    assert_eq!(rest.anchor, changes.anchor);

    let seen = changes.anchor;
    writer.move_to(beach, None, "beach.jpg").await.unwrap();
    writer.delete(photos).await.unwrap();
    synced(&mut reader).await;
    let moved = local.changes(&seen.to_bytes(), 100).unwrap();
    assert_eq!(moved.updated.len(), 1);
    assert_eq!(moved.updated[0].parent, None);
    assert_eq!(moved.removed, [photos]);
    assert!(local.item(&photos).unwrap().is_none());

    let elsewhere = store(&drive, &[&a]);
    assert!(matches!(
        local.changes(&elsewhere.anchor().unwrap().to_bytes(), 10),
        Err(Error::AnchorExpired)
    ));
    assert!(matches!(local.changes(b"", 10), Err(Error::AnchorExpired)));
}

#[tokio::test]
async fn following_reports_changes_until_stopped() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let local = store(&drive, &[&a]);
    let mut follower = open(&drive, &local).await;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (seen, mut changes) = tokio::sync::mpsc::unbounded_channel();

    let following = tokio::spawn(async move {
        follower
            .follow(
                async {
                    stopped.await.ok();
                },
                &Notify::new(),
                |event| {
                    if matches!(event, Followed::ItemsChanged) {
                        seen.send(()).unwrap();
                    }
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut writer = device(&drive, &[&a]).await;
    let (node, _) = writer.create_folder(None, "Live").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("the change is reported");
    assert_eq!(local.item(&node).unwrap().unwrap().name, "Live");

    stop.send(()).unwrap();
    following.await.unwrap().unwrap();
}

fn extension(drive: &DriveKeys, store: &Arc<Store>) -> ItemWriter<MemoryBlobStore> {
    let keys = DriveKeys::from_secret(drive.keys().secret_key().clone());
    ItemWriter::new(Arc::clone(store), keys, MemoryBlobStore::new()).unwrap()
}

fn new_item(parent: Option<NodeId>, name: &str, folder: bool) -> NewItem {
    NewItem {
        request: format!("{parent:?}/{name}"),
        place: Place {
            parent,
            name: name.into(),
        },
        folder,
        may_exist: false,
        created: None,
        modified: None,
    }
}

async fn wait_for(mut done: impl AsyncFnMut() -> bool) {
    for _ in 0..500 {
        if done().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

#[tokio::test]
async fn the_session_takes_in_and_sends_what_the_extension_wrote() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let local = store(&drive, &[&a]);
    let mut session = open(&drive, &local).await;
    let writer = extension(&drive, &local);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beach.jpg");
    std::fs::write(&path, b"jpg").unwrap();

    let photos = writer
        .create(new_item(None, "Photos", true), None, &|_| {})
        .await
        .unwrap()
        .unwrap();
    let new = new_item(Some(photos.node), "beach.jpg", false);
    writer.create(new, Some(&path), &|_| {}).await.unwrap();
    let anchor = local.anchor().unwrap();
    assert_eq!(a.wraps().await, 0, "the extension sends nothing");

    assert_eq!(session.take_in().await.unwrap(), 2);
    assert_eq!(a.wraps().await, 2);
    assert!(local.outbox(0).unwrap().is_empty());
    assert_eq!(listing(&session), ["Photos", "Photos/beach.jpg"]);
    let changes = local.changes(&anchor.to_bytes(), 10).unwrap();
    assert!(changes.updated.is_empty(), "the extension's items stand");

    let mut other = device(&drive, &[&a]).await;
    synced(&mut other).await;
    assert_eq!(listing(&other), ["Photos", "Photos/beach.jpg"]);

    writer.delete(photos.node, None, true).unwrap();
    assert_eq!(synced(&mut session).await.sent, 2);
    assert!(session.tree().is_empty());
    synced(&mut other).await;
    assert!(other.tree().is_empty());
    assert_eq!(other.settle().await.unwrap(), 0, "no folder revived");
}

#[tokio::test]
async fn the_tree_rules_correct_what_the_extension_wrote() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let mut other = device(&drive, &[&a]).await;
    other.create_folder(None, "report").await.unwrap();
    let local = store(&drive, &[&a]);
    let mut session = open(&drive, &local).await;
    synced(&mut session).await;

    let writer = extension(&drive, &local);
    let created = writer
        .create(new_item(None, "Report", true), None, &|_| {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created.name, "Report");
    session.take_in().await.unwrap();

    let mut names: Vec<String> = local
        .children(None, None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.name)
        .collect();
    names.sort();
    assert_eq!(names, listing(&session));
    assert!(names.iter().any(|name| name.ends_with(" (2)")));
}

#[tokio::test]
async fn following_takes_in_what_the_extension_wrote() {
    let a = TestRelay::start().await;
    let drive = DriveKeys::generate();
    let local = store(&drive, &[&a]);
    let mut follower = open(&drive, &local).await;
    let written = Arc::new(Notify::new());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let wake = Arc::clone(&written);
    let following = tokio::spawn(async move {
        follower
            .follow(
                async {
                    stopped.await.ok();
                },
                &wake,
                |_| {},
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let writer = extension(&drive, &local);
    writer
        .create(new_item(None, "From Finder", true), None, &|_| {})
        .await
        .unwrap();
    written.notify_one();
    wait_for(async || a.wraps().await == 1).await;
    wait_for(async || local.outbox(0).unwrap().is_empty()).await;

    stop.send(()).unwrap();
    following.await.unwrap().unwrap();
}
