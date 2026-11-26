//! Relay transport over nostr-sdk. Drive relays are reached as the drive
//! key, which answers NIP-42 AUTH so that relays following NIP-17 serve the
//! drive its wraps. The user's relays, where descriptors live, are reached
//! as the user.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::join_all;
use futures::{Stream, StreamExt};
use nostr::event::{Event, EventId, FinalizeEventAsync};
use nostr::filter::Filter;
use nostr::key::Keys;
use nostr::message::SubscriptionId;
use nostr::nips::nip42::ClientAuthentication;
use nostr::types::{RelayUrl, Timestamp};
use nostr_sdk::authenticator::{Authenticator, SignerAuthenticator};
use nostr_sdk::client::{Client, ClientNotification};
use nostr_sdk::error::{Error as SdkError, ErrorKind};
use nostr_sdk::relay::{Relay, SyncOptions};

use crate::descriptor::UserSigner;
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::task;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_BATCH: usize = 100;
const PAGE: usize = 500;
const AUTH_ATTEMPTS: u32 = 4;

/// How a relay's events, among those a filter selects, compare with ours.
#[derive(Debug, Default)]
pub struct Comparison {
    /// Every compared event the relay holds, ours or not.
    pub held: HashSet<EventId>,
    /// Held by the relay only, still to download.
    pub missing: Vec<EventId>,
    /// Held by the relay only and already downloaded, from relays without
    /// negentropy.
    pub events: Vec<Event>,
}

#[derive(Debug, Default)]
pub struct Sent {
    pub accepted: Vec<RelayUrl>,
    pub failed: Vec<(RelayUrl, String)>,
}

impl Sent {
    /// Why each relay refused, for display.
    pub fn reasons(&self) -> String {
        let reasons: Vec<String> = self
            .failed
            .iter()
            .map(|(url, reason)| format!("{url}: {reason}"))
            .collect();
        reasons.join("; ")
    }
}

pub struct Relays {
    client: Client,
    urls: Vec<RelayUrl>,
    /// Listed with plain REQs from then on.
    without_negentropy: Mutex<HashSet<RelayUrl>>,
}

impl Relays {
    /// Signs AUTH with the drive key.
    pub async fn for_drive(
        keys: &DriveKeys,
        urls: impl IntoIterator<Item = RelayUrl>,
    ) -> Result<Self> {
        Self::for_keys(keys.keys(), urls).await
    }

    /// Signs AUTH with `keys`, such as a share link's reader key.
    pub async fn for_keys(keys: &Keys, urls: impl IntoIterator<Item = RelayUrl>) -> Result<Self> {
        Self::connect(authenticated(SignerAuthenticator::new(keys.clone())), urls).await
    }

    /// Signs AUTH with the user's key, when a relay asks.
    pub async fn for_user<S: UserSigner>(
        signer: Arc<S>,
        urls: impl IntoIterator<Item = RelayUrl>,
    ) -> Result<Self> {
        Self::connect(authenticated(UserAuthenticator(signer)), urls).await
    }

    /// Without AUTH, for public lists and events already signed.
    pub async fn public(urls: impl IntoIterator<Item = RelayUrl>) -> Result<Self> {
        Self::connect(Client::builder().build(), urls).await
    }

    async fn connect(client: Client, urls: impl IntoIterator<Item = RelayUrl>) -> Result<Self> {
        install_tls_provider();
        let mut unique = Vec::new();
        for url in urls {
            if !unique.contains(&url) {
                unique.push(url);
            }
        }
        if unique.is_empty() {
            return Err(Error::Malformed("no relay to connect to"));
        }
        for url in &unique {
            client
                .add_relay(url)
                .await
                .map_err(|e| relay_error(url, e))?;
        }
        client.connect().and_wait(CONNECT_TIMEOUT).await;
        Ok(Self {
            client,
            urls: unique,
            without_negentropy: Mutex::default(),
        })
    }

    pub fn urls(&self) -> &[RelayUrl] {
        &self.urls
    }

    /// Negentropy (NIP-77) in dry run: only ids travel. Relays without it
    /// are listed with plain REQs instead.
    pub async fn compare(
        &self,
        url: &RelayUrl,
        filter: &Filter,
        items: &[(EventId, Timestamp)],
    ) -> Result<Comparison> {
        let relay = self.relay(url).await?;
        let mut attempt = 1;
        while !self.lacks_negentropy(url) {
            let sync = relay
                .sync(filter.clone())
                .items(items.iter().copied())
                .opts(SyncOptions::new().dry_run())
                .await;
            match sync {
                Ok(summary) => {
                    let mut held: HashSet<EventId> = items
                        .iter()
                        .map(|(id, _)| *id)
                        .filter(|id| !summary.local.contains(id))
                        .collect();
                    held.extend(summary.remote.iter().copied());
                    return Ok(Comparison {
                        held,
                        missing: summary.remote.into_iter().collect(),
                        events: Vec::new(),
                    });
                }
                // The relay sent its AUTH challenge with this refusal.
                Err(e) if e.to_string().contains("auth-required") && attempt < AUTH_ATTEMPTS => {
                    task::sleep(Duration::from_millis(100) * 2u32.pow(attempt)).await;
                    attempt += 1;
                }
                Err(e) if refuses_negentropy(&e) => {
                    self.without_negentropy.lock().unwrap().insert(url.clone());
                }
                Err(e) => return Err(relay_error(url, e)),
            }
        }
        let ours: HashSet<EventId> = items.iter().map(|(id, _)| *id).collect();
        let listed = list(&relay, url, filter).await?;
        Ok(Comparison {
            held: listed.iter().map(|e| e.id).collect(),
            missing: Vec::new(),
            events: listed
                .into_iter()
                .filter(|e| !ours.contains(&e.id))
                .collect(),
        })
    }

    fn lacks_negentropy(&self, url: &RelayUrl) -> bool {
        self.without_negentropy.lock().unwrap().contains(url)
    }

    /// Events by id, within `filter`: relays that follow NIP-17 serve gift
    /// wraps only to filters naming their recipient.
    pub async fn fetch(
        &self,
        url: &RelayUrl,
        filter: &Filter,
        ids: &[EventId],
    ) -> Result<Vec<Event>> {
        let relay = self.relay(url).await?;
        let mut events = Vec::with_capacity(ids.len());
        for batch in ids.chunks(FETCH_BATCH) {
            let filter = filter.clone().ids(batch.iter().copied());
            events.extend(fetch_all(&relay, url, filter).await?);
        }
        Ok(events)
    }

    /// The events matching `filter` on the relay `url`, one page of them.
    pub async fn events(&self, url: &RelayUrl, filter: &Filter) -> Result<Vec<Event>> {
        let relay = self.relay(url).await?;
        fetch_all(&relay, url, filter.clone().limit(PAGE)).await
    }

    /// Every event matching `filter` on every relay, deduplicated. Fails
    /// only when no relay answers.
    pub async fn query(&self, filter: &Filter) -> Result<Vec<Event>> {
        let listings = join_all(self.urls.iter().map(|url| async move {
            let relay = self.relay(url).await?;
            list(&relay, url, filter).await
        }))
        .await;
        let mut events = HashMap::new();
        let mut error = None;
        let mut answered = false;
        for listing in listings {
            match listing {
                Ok(listed) => {
                    answered = true;
                    events.extend(listed.into_iter().map(|e| (e.id, e)));
                }
                Err(e) => error = Some(e),
            }
        }
        match (answered, error) {
            (false, Some(e)) => Err(e),
            _ => Ok(events.into_values().collect()),
        }
    }

    /// The relays that do not hold `event`, after asking each.
    pub async fn lacking(&self, event: &Event) -> Vec<RelayUrl> {
        let filter = Filter::new()
            .id(event.id)
            .kind(event.kind)
            .author(event.pubkey);
        let held = join_all(self.urls.iter().map(|url| {
            let filter = filter.clone();
            async move {
                let relay = self.relay(url).await.ok()?;
                let found = fetch_all(&relay, url, filter).await.ok()?;
                Some(!found.is_empty())
            }
        }))
        .await;
        self.urls
            .iter()
            .zip(held)
            .filter(|(_, held)| *held == Some(false))
            .map(|(url, _)| url.clone())
            .collect()
    }

    pub async fn send(&self, event: &Event, to: &[RelayUrl]) -> Sent {
        if to.is_empty() {
            return Sent::default();
        }
        match self.client.send_event(event).to(to).await {
            Ok(output) => Sent {
                accepted: output.success.into_keys().collect(),
                failed: output.failed.into_iter().collect(),
            },
            Err(e) => Sent {
                accepted: Vec::new(),
                failed: to.iter().map(|url| (url.clone(), e.to_string())).collect(),
            },
        }
    }

    pub async fn send_all(&self, event: &Event) -> Sent {
        self.send(event, &self.urls).await
    }

    /// New events only, from every relay, each event once.
    pub async fn subscribe(
        &self,
        filters: Vec<Filter>,
    ) -> Result<impl Stream<Item = Event> + Send + use<>> {
        let notifications = self.client.notifications();
        let id = SubscriptionId::generate();
        let filters: Vec<Filter> = filters.into_iter().map(|f| f.limit(0)).collect();
        self.client
            .subscribe(filters)
            .with_id(id.clone())
            .await
            .map_err(|e| Error::Nostr(e.to_string()))?;
        Ok(notifications.filter_map(move |notification| {
            let event = match notification {
                ClientNotification::Event {
                    subscription_id,
                    event,
                    ..
                } if subscription_id == id => Some(*event),
                _ => None,
            };
            async move { event }
        }))
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }

    async fn relay(&self, url: &RelayUrl) -> Result<Relay> {
        self.client
            .relay(url)
            .await
            .map_err(|e| relay_error(url, e))?
            .ok_or_else(|| Error::Relay {
                relay: url.to_string(),
                message: "not a drive relay".into(),
            })
    }
}

fn authenticated(authenticator: impl Authenticator + 'static) -> Client {
    Client::builder().authenticator(authenticator).build()
}

/// nostr-sdk always brings rustls' ring provider and the core asks for
/// aws-lc-rs: with both, rustls picks neither and every TLS connection panics.
#[cfg(not(target_family = "wasm"))]
pub(crate) fn install_tls_provider() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[cfg(target_family = "wasm")]
pub(crate) fn install_tls_provider() {}

/// The events matching `filter` once the relay said it sent them all.
/// nostr-sdk returns what came so far, without an error, when its own
/// timeout passes or the relay disconnects: nothing from a relay that never
/// answered.
async fn fetch_all(relay: &Relay, url: &RelayUrl, filter: Filter) -> Result<Vec<Event>> {
    let Some(found) = task::timeout(FETCH_TIMEOUT, relay.fetch_events(filter)).await else {
        return Err(relay_error(url, "no answer in time"));
    };
    let found = found.map_err(|e| relay_error(url, e))?;
    if !relay.status().is_connected() {
        return Err(relay_error(url, "disconnected before answering"));
    }
    Ok(found.into_iter().collect())
}

/// Pages back through `created_at` until a page brings nothing new.
async fn list(relay: &Relay, url: &RelayUrl, filter: &Filter) -> Result<Vec<Event>> {
    let mut events: HashMap<EventId, Event> = HashMap::new();
    let mut until = None;
    loop {
        let mut page = filter.clone().limit(PAGE);
        if let Some(until) = until {
            page = page.until(until);
        }
        let found = fetch_all(relay, url, page).await?;
        let oldest = found.iter().map(|e| e.created_at).min();
        let new = found
            .into_iter()
            .filter(|e| events.insert(e.id, e.clone()).is_none())
            .count();
        match oldest {
            Some(oldest) if new > 0 => until = Some(oldest),
            _ => break,
        }
    }
    Ok(events.into_values().collect())
}

/// A relay that ignores NEG-OPEN, rather than refusing it, only times out.
fn refuses_negentropy(error: &SdkError) -> bool {
    matches!(error.kind(), ErrorKind::Unsupported | ErrorKind::Timeout)
        || error.to_string().contains("negentropy")
}

fn relay_error(url: &RelayUrl, error: impl fmt::Display) -> Error {
    Error::Relay {
        relay: url.to_string(),
        message: error.to_string(),
    }
}

/// NIP-42 with the user's signer, which may be remote (NIP-46).
struct UserAuthenticator<S>(Arc<S>);

impl<S> fmt::Debug for UserAuthenticator<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UserAuthenticator")
    }
}

impl<S: UserSigner> Authenticator for UserAuthenticator<S> {
    fn make_auth_event<'a>(
        &'a self,
        relay_url: &'a RelayUrl,
        challenge: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Event, SdkError>> + Send + 'a>> {
        Box::pin(async move {
            ClientAuthentication::new(challenge, relay_url.clone())
                .finalize_async(&*self.0)
                .await
                .map_err(SdkError::from)
        })
    }
}
