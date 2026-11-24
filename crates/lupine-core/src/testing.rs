//! Test relays that behave as drive relays should.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use nostr::event::{Event, EventId, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::types::{RelayUrl, Timestamp};
use nostr_database::error::Error as DatabaseError;
use nostr_database::{
    DatabaseEventStatus, Features, NostrDatabase, RejectedReason, SaveEventStatus,
};
use nostr_memory::MemoryDatabase;
use nostr_sdk::local_relay::{LocalRelay, LocalRelayBuilder};

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T, DatabaseError>> + Send + 'a>>;

/// A relay store that applies NIP-59: a deletion from a gift wrap's
/// recipient deletes it. nostr-memory 0.45 applies only NIP-09's author
/// rule, and refuses such deletions outright.
#[derive(Debug)]
pub(crate) struct Nip59Store {
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
pub(crate) struct TestRelay {
    pub relay: LocalRelay,
    pub store: Arc<Nip59Store>,
    pub url: RelayUrl,
}

impl TestRelay {
    pub async fn start() -> Self {
        Self::with(|builder| builder).await
    }

    pub async fn with(configure: impl FnOnce(LocalRelayBuilder) -> LocalRelayBuilder) -> Self {
        let store = Arc::new(Nip59Store::new());
        let builder = LocalRelay::builder()
            .database(Arc::clone(&store))
            .auth_dm(true);
        let relay = configure(builder).build();
        relay.run().await.unwrap();
        let url = relay.url().await;
        Self { relay, store, url }
    }

    pub async fn holds(&self, id: EventId) -> bool {
        self.store.event_by_id(&id).await.unwrap().is_some()
    }

    pub async fn wraps(&self) -> usize {
        let filter = Filter::new().kind(Kind::GiftWrap);
        self.store.count(filter).await.unwrap()
    }
}

/// A Blossom server that answers every request with `status`, 200 by
/// default: enough for the upload checks of tests that never upload.
pub(crate) async fn blossom_server(status: Option<u16>) -> String {
    let status = axum::http::StatusCode::from_u16(status.unwrap_or(200)).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().fallback(move || async move { status });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}
