//! Share links. A link names a mailbox on relays, not the file: the drive
//! posts what the link shows there as NIP-59 gift wraps, and stopping the
//! link takes it down. See `docs/protocol.md`.

use std::cmp::Reverse;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::join_all;
use nostr::event::{
    Event, EventBuilder, EventId, FinalizeEvent, FinalizeUnsignedEvent, IntoEventBuilder, Kind, Tag,
};
use nostr::filter::Filter;
use nostr::key::{Keys, PublicKey, SecretKey};
use nostr::nips::nip09::EventDeletionRequest;
use nostr::nips::nip59::{GiftWrapBuilder, UnwrappedGift};
use nostr::types::{RelayUrl, Timestamp};
use serde::{Deserialize, Serialize};

use crate::encoding;
use crate::error::{Error, Result};
use crate::graph::History;
use crate::ids::NodeId;
use crate::names;
use crate::node::{FileMeta, NodeType, unix_millis};
use crate::relays::Relays;
use crate::store::Store;

pub const MANIFEST_KIND: Kind = Kind::Custom(crate::kinds::SHARE_MANIFEST);
const FORMAT_VERSION: u8 = 1;
const TLV_READER: u8 = 0;
const TLV_OWNER: u8 = 1;
const TLV_RELAY: u8 = 2;
const DELETION_BATCH: usize = 100;

/// What the drive keeps of a link, in its share node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareMeta {
    /// The link carries it: readers answer AUTH with it, and the link's
    /// wraps are addressed to its pubkey.
    #[serde(with = "encoding::secret_key")]
    pub reader: SecretKey,
    /// Only the drive holds it: it seals what the link shows.
    #[serde(with = "encoding::secret_key")]
    pub owner: SecretKey,
    pub relays: Vec<RelayUrl>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
}

impl ShareMeta {
    pub fn link(&self) -> ShareLink {
        ShareLink {
            reader: self.reader.clone(),
            owner: Keys::new(self.owner.clone()).public_key(),
            relays: self.relays.clone(),
        }
    }

    pub fn expired(&self, now_secs: u64) -> bool {
        self.expires.is_some_and(|expires| expires <= now_secs)
    }
}

/// A live link of the drive, as its share node holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub node: NodeId,
    /// The share node's name.
    pub label: String,
    /// Unix milliseconds.
    pub created: Option<i64>,
    pub file: NodeId,
    pub meta: ShareMeta,
}

/// The fragment of a share link: the base64url encoding, without padding,
/// of a NIP-19 style TLV list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareLink {
    pub reader: SecretKey,
    pub owner: PublicKey,
    pub relays: Vec<RelayUrl>,
}

impl ShareLink {
    /// Leaves out relay URLs longer than 255 bytes.
    pub fn encode(&self) -> String {
        let mut tlv = Vec::new();
        let mut push = |kind: u8, value: &[u8]| {
            let len = u8::try_from(value.len()).expect("TLV values fit in 255 bytes");
            tlv.extend([kind, len]);
            tlv.extend_from_slice(value);
        };
        push(TLV_READER, self.reader.as_secret_bytes());
        push(TLV_OWNER, self.owner.as_bytes());
        for relay in &self.relays {
            let url = relay.as_str_without_trailing_slash();
            if url.len() <= usize::from(u8::MAX) {
                push(TLV_RELAY, url.as_bytes());
            }
        }
        URL_SAFE_NO_PAD.encode(tlv)
    }

    pub fn decode(text: &str) -> Result<Self> {
        let malformed = || Error::Malformed("invalid share link");
        let tlv = URL_SAFE_NO_PAD
            .decode(text.trim().trim_end_matches('='))
            .map_err(|_| malformed())?;
        let (mut reader, mut owner, mut relays) = (None, None, Vec::new());
        let mut rest = tlv.as_slice();
        while let [kind, len, tail @ ..] = rest {
            let len = usize::from(*len);
            if tail.len() < len {
                return Err(malformed());
            }
            let (value, next) = tail.split_at(len);
            rest = next;
            match *kind {
                TLV_READER => reader = Some(SecretKey::from_slice(value).map_err(|_| malformed())?),
                TLV_OWNER => owner = Some(PublicKey::from_slice(value).map_err(|_| malformed())?),
                TLV_RELAY => {
                    let url = std::str::from_utf8(value).map_err(|_| malformed())?;
                    relays.push(RelayUrl::parse(url).map_err(|_| malformed())?);
                }
                _ => {}
            }
        }
        if !rest.is_empty() || relays.is_empty() {
            return Err(malformed());
        }
        Ok(Self {
            reader: reader.ok_or_else(malformed)?,
            owner: owner.ok_or_else(malformed)?,
            relays,
        })
    }
}

/// What a live link shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    pub name: String,
    /// In BUD-03 order, then the servers the drive is leaving.
    pub servers: Vec<String>,
    pub file: FileMeta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Live {
        shown: Shown,
        expires: Option<u64>,
    },
    Expired,
    /// Its sharing stopped: final.
    Revoked,
    /// No manifest on any relay of the link.
    Empty,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    v: u8,
    /// Unix milliseconds.
    time: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    servers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<FileMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    revoked: bool,
}

impl Manifest {
    fn new(shown: Option<&Shown>, expires: Option<u64>) -> Self {
        Self {
            v: FORMAT_VERSION,
            time: unix_millis(),
            name: shown.map(|s| s.name.clone()),
            servers: shown.map(|s| s.servers.clone()),
            file: shown.map(|s| s.file.clone()),
            expires: shown.and(expires),
            revoked: shown.is_none(),
        }
    }

    fn shown(&self) -> Option<Shown> {
        let name = self.name.clone().filter(|n| names::validate(n).is_ok())?;
        let servers = self.servers.clone().filter(|s| !s.is_empty())?;
        Some(Shown {
            name,
            servers,
            file: self.file.clone()?,
        })
    }
}

/// The state that the manifests among `wraps` give the link: a revoked one
/// stops it whatever its time, otherwise the latest wins. Wraps that the
/// owner key did not seal are ignored: anyone who has the link can post.
pub fn state(link: &ShareLink, wraps: &[Event], now_secs: u64) -> LinkState {
    let reader = Keys::new(link.reader.clone());
    let mut latest: Option<(i64, Reverse<EventId>, Shown, Option<u64>)> = None;
    for wrap in wraps {
        let Some((id, manifest)) = unwrap(&reader, &link.owner, wrap) else {
            continue;
        };
        if manifest.revoked {
            return LinkState::Revoked;
        }
        let Some(shown) = manifest.shown() else {
            continue;
        };
        let key = (manifest.time, Reverse(id));
        if latest
            .as_ref()
            .is_none_or(|(time, id, ..)| key > (*time, *id))
        {
            latest = Some((key.0, key.1, shown, manifest.expires));
        }
    }
    match latest {
        None => LinkState::Empty,
        Some((.., expires)) if expires.is_some_and(|e| e <= now_secs) => LinkState::Expired,
        Some((_, _, shown, expires)) => LinkState::Live { shown, expires },
    }
}

fn unwrap(reader: &Keys, owner: &PublicKey, wrap: &Event) -> Option<(EventId, Manifest)> {
    let gift = UnwrappedGift::from_gift_wrap(reader, wrap).ok()?;
    if gift.sender != *owner || gift.rumor.kind != MANIFEST_KIND {
        return None;
    }
    let manifest: Manifest = serde_json::from_str(&gift.rumor.content).ok()?;
    let id = gift.rumor.id?;
    (manifest.v == FORMAT_VERSION).then_some((id, manifest))
}

/// `shown`, or a revoked manifest when `None`, sealed by the owner key for
/// the reader key. A live manifest that expires carries the time in an
/// `expiration` tag, so that relays may drop it. A revoked one stays.
fn wrap(meta: &ShareMeta, shown: Option<&Shown>) -> Result<Event> {
    let owner = Keys::new(meta.owner.clone());
    let reader = Keys::new(meta.reader.clone()).public_key();
    let manifest = Manifest::new(shown, meta.expires);
    let rumor = EventBuilder::new(MANIFEST_KIND, serde_json::to_string(&manifest)?)
        .finalize_unsigned(owner.public_key());
    let expiration = manifest
        .expires
        .map(|secs| Tag::expiration(Timestamp::from_secs(secs)));
    GiftWrapBuilder::new(reader, rumor)
        .extra_tags(expiration)
        .finalize(&owner)
        .map_err(nostr_error)
}

/// A link's relays, reached as its reader key.
struct Mailbox {
    reader: Keys,
    relays: Relays,
}

impl Mailbox {
    async fn open(reader: &SecretKey, relays: &[RelayUrl]) -> Result<Self> {
        let reader = Keys::new(reader.clone());
        let relays = Relays::for_keys(&reader, relays.iter().cloned()).await?;
        Ok(Self { reader, relays })
    }

    fn filter(&self) -> Filter {
        Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.reader.public_key())
    }

    /// Fails only when no relay answers.
    async fn wraps(&self) -> Result<Vec<Event>> {
        self.relays.query(&self.filter()).await
    }

    /// As their recipient, which NIP-59 has relays honor whatever key
    /// signed the wraps.
    async fn delete(&self, wraps: &[EventId]) -> Result<()> {
        for batch in wraps.chunks(DELETION_BATCH) {
            let kind = Tag::parse(["k", "1059"]).map_err(nostr_error)?;
            let deletion = EventDeletionRequest::new()
                .ids(batch.iter().copied())
                .into_event_builder()
                .tag(kind)
                .finalize(&self.reader)
                .map_err(nostr_error)?;
            self.relays.send_all(&deletion).await;
        }
        Ok(())
    }

    async fn shutdown(self) {
        self.relays.shutdown().await;
    }
}

/// What the link shows, read from its relays.
pub async fn open(link: &ShareLink) -> Result<LinkState> {
    let mailbox = Mailbox::open(&link.reader, &link.relays).await?;
    let wraps = mailbox.wraps().await;
    mailbox.shutdown().await;
    Ok(state(link, &wraps?, now_secs()))
}

/// A new link that shows `shown`, on those of `relays` that take its first
/// manifest and serve it back to the reader key: some relays take gift
/// wraps only for their members. Write its share node next.
pub async fn create(relays: &[RelayUrl], shown: &Shown, expires: Option<u64>) -> Result<ShareMeta> {
    let mut meta = ShareMeta {
        reader: SecretKey::generate(),
        owner: SecretKey::generate(),
        relays: relays.to_vec(),
        expires,
    };
    let first = wrap(&meta, Some(shown))?;
    let mailbox = Mailbox::open(&meta.reader, relays).await?;
    let sent = mailbox.relays.send_all(&first).await;
    let filter = mailbox.filter().id(first.id);
    let served = join_all(relays.iter().map(|url| async {
        let taken = sent.accepted.contains(url);
        let served = taken
            && mailbox
                .relays
                .events(url, &filter)
                .await
                .is_ok_and(|found| !found.is_empty());
        served.then(|| url.clone())
    }))
    .await;
    mailbox.shutdown().await;
    meta.relays = served.into_iter().flatten().collect();
    if meta.relays.is_empty() {
        let reasons = sent.reasons();
        return Err(Error::NotPublished(if reasons.is_empty() {
            "no drive relay serves share links".into()
        } else {
            reasons
        }));
    }
    Ok(meta)
}

/// Shows `shown` on the link from now on, or stops it for good when `None`,
/// then deletes the other wraps to the reader key: older manifests, and
/// what others posted. A revoked manifest stays, and nothing is shown
/// after one: a device that misses a stop cannot undo it.
pub async fn publish(meta: &ShareMeta, shown: Option<&Shown>) -> Result<()> {
    let wrap = wrap(meta, shown)?;
    let mailbox = Mailbox::open(&meta.reader, &meta.relays).await?;
    let published = async {
        let reader = Keys::new(meta.reader.clone());
        let owner = Keys::new(meta.owner.clone()).public_key();
        let revoked =
            |event: &Event| unwrap(&reader, &owner, event).is_some_and(|(_, m)| m.revoked);
        let before = mailbox.wraps().await?;
        if shown.is_some() && before.iter().any(revoked) {
            return Err(Error::Malformed("the link was stopped"));
        }
        let sent = mailbox.relays.send_all(&wrap).await;
        if sent.accepted.is_empty() {
            return Err(Error::NotPublished(sent.reasons()));
        }
        let others: Vec<EventId> = before
            .iter()
            .filter(|event| event.id != wrap.id && !revoked(event))
            .map(|event| event.id)
            .collect();
        mailbox.delete(&others).await
    }
    .await;
    mailbox.shutdown().await;
    published
}

/// What the link's relays show now.
pub async fn posted(meta: &ShareMeta) -> Result<LinkState> {
    open(&meta.link()).await
}

/// The live links of `file` in the local database, oldest first.
pub fn shares_of(store: &Store, file: &NodeId) -> Result<Vec<Share>> {
    let mut shares = Vec::new();
    for node in store.share_nodes(file)? {
        shares.extend(share(store, &node)?);
    }
    shares.sort_by_key(|share| share.created);
    Ok(shares)
}

/// The link `node` holds, when it is a live share.
pub fn share(store: &Store, node: &NodeId) -> Result<Option<Share>> {
    let versions = store.node_versions(node)?;
    let history = History::new(versions.iter().map(|(id, version)| (*id, version)));
    let Some(heads) = history.heads() else {
        return Ok(None);
    };
    let (_, winner) = heads.winner();
    Ok(match (winner.node_type, &winner.share, winner.parent) {
        (NodeType::Share, Some(meta), Some(file)) => Some(Share {
            node: *node,
            label: winner.name.clone(),
            created: winner.created,
            file,
            meta: meta.clone(),
        }),
        _ => None,
    })
}

pub(crate) fn now_secs() -> u64 {
    (unix_millis() / 1000) as u64
}

fn nostr_error(e: impl std::fmt::Display) -> Error {
    Error::Nostr(e.to_string())
}

#[cfg(test)]
mod tests;
