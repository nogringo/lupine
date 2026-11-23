//! The drive descriptor (kind 3540): the drive secret, encrypted by the
//! user's identity to itself, so any device signed in as the user finds the
//! drive. And the recovery key, which restores a drive without the user's
//! key.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;

use bech32::{Bech32, Hrp};
use nostr::event::{AsyncSignEvent, Event, EventBuilder, EventId, FinalizeEventAsync, Kind, Tag};
use nostr::key::{AsyncGetPublicKey, SecretKey};
use nostr::nips::nip44::AsyncNip44;
use nostr::types::RelayUrl;
use serde::{Deserialize, Serialize};

use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::ids::DriveId;

pub const DESCRIPTOR_KIND: Kind = Kind::Custom(crate::kinds::DRIVE_DESCRIPTOR);
const FORMAT_VERSION: u8 = 1;
const RECOVERY_HRP: Hrp = Hrp::parse_unchecked("lupine");

/// What a user signer must do for drives: read and write descriptors,
/// which NIP-46 bunkers and NIP-07 extensions can.
pub trait UserSigner: AsyncGetPublicKey + AsyncSignEvent + AsyncNip44 {}

impl<T: AsyncGetPublicKey + AsyncSignEvent + AsyncNip44> UserSigner for T {}

#[derive(Clone)]
pub struct Descriptor {
    pub id: DriveId,
    /// Key rotations so far. The drive key changes with each.
    pub epoch: u32,
    pub secret: SecretKey,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct Json {
    v: u8,
    id: DriveId,
    epoch: u32,
    secret: String,
    name: String,
}

impl Descriptor {
    pub fn generate(name: &str) -> Self {
        Self {
            id: DriveId::random(),
            epoch: 0,
            secret: SecretKey::generate(),
            name: name.to_owned(),
        }
    }

    pub fn keys(&self) -> DriveKeys {
        DriveKeys::from_secret(self.secret.clone())
    }

    /// The descriptor event, encrypted by the user to themselves.
    pub async fn seal<S: UserSigner>(&self, signer: &S) -> Result<Event> {
        let user = signer.get_public_key_async().await.map_err(nostr_error)?;
        let json = serde_json::to_string(&Json {
            v: FORMAT_VERSION,
            id: self.id,
            epoch: self.epoch,
            secret: self.secret.to_secret_hex(),
            name: self.name.clone(),
        })?;
        let content = signer
            .nip44_encrypt_async(&user, &json)
            .await
            .map_err(nostr_error)?;
        EventBuilder::new(DESCRIPTOR_KIND, content)
            .finalize_async(signer)
            .await
            .map_err(nostr_error)
    }

    /// `None` for events that are not a descriptor of this user in a
    /// format this client reads. Errors come from the signer.
    pub async fn open<S: UserSigner>(signer: &S, event: &Event) -> Result<Option<Self>> {
        let user = signer.get_public_key_async().await.map_err(nostr_error)?;
        if event.kind != DESCRIPTOR_KIND || event.pubkey != user || event.verify().is_err() {
            return Ok(None);
        }
        let json = signer
            .nip44_decrypt_async(&user, &event.content)
            .await
            .map_err(nostr_error)?;
        let Ok(json) = serde_json::from_str::<Json>(&json) else {
            return Ok(None);
        };
        let Ok(secret) = SecretKey::from_hex(&json.secret) else {
            return Ok(None);
        };
        Ok((json.v == FORMAT_VERSION).then_some(Self {
            id: json.id,
            epoch: json.epoch,
            secret,
            name: json.name,
        }))
    }

    pub fn recovery_key(&self, relays: &[RelayUrl]) -> RecoveryKey {
        RecoveryKey {
            secret: self.secret.clone(),
            id: self.id,
            epoch: self.epoch,
            relays: relays.to_vec(),
        }
    }
}

impl fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Descriptor")
            .field("id", &self.id)
            .field("epoch", &self.epoch)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A descriptor and the event it came in, kept to republish it unchanged.
#[derive(Clone, Debug)]
pub struct FoundDrive {
    pub event: Event,
    pub descriptor: Descriptor,
}

/// The current descriptor of each drive: highest epoch, then latest
/// `created_at`, then lowest event id. The others are the ones to delete.
pub fn current(found: impl IntoIterator<Item = FoundDrive>) -> (Vec<FoundDrive>, Vec<EventId>) {
    let mut by_drive: HashMap<DriveId, Vec<FoundDrive>> = HashMap::new();
    for drive in found {
        by_drive.entry(drive.descriptor.id).or_default().push(drive);
    }
    let mut current = Vec::new();
    let mut superseded = Vec::new();
    for (_, mut descriptors) in by_drive {
        descriptors.sort_by_key(|d| (d.descriptor.epoch, d.event.created_at, Reverse(d.event.id)));
        descriptors.dedup_by_key(|d| d.event.id);
        let latest = descriptors
            .pop()
            .expect("grouped descriptors are never empty");
        superseded.extend(descriptors.into_iter().map(|d| d.event.id));
        current.push(latest);
    }
    current.sort_by(|a, b| a.descriptor.name.cmp(&b.descriptor.name));
    (current, superseded)
}

/// NIP-09 deletion of the user's own descriptors, by the user.
pub async fn deletion<S: UserSigner>(signer: &S, descriptors: &[EventId]) -> Result<Event> {
    let kind =
        Tag::parse(["k", &crate::kinds::DRIVE_DESCRIPTOR.to_string()]).map_err(nostr_error)?;
    EventBuilder::new(Kind::EventDeletion, "")
        .tags(descriptors.iter().copied().map(Tag::event))
        .tag(kind)
        .finalize_async(signer)
        .await
        .map_err(nostr_error)
}

/// Full access to a drive without the user's key: the bech32 encoding,
/// prefix `lupine`, of a NIP-19 style TLV list.
#[derive(Clone)]
pub struct RecoveryKey {
    pub secret: SecretKey,
    pub id: DriveId,
    pub epoch: u32,
    pub relays: Vec<RelayUrl>,
}

const TLV_SECRET: u8 = 0;
const TLV_ID: u8 = 1;
const TLV_EPOCH: u8 = 2;
const TLV_RELAY: u8 = 3;

impl RecoveryKey {
    pub fn encode(&self) -> String {
        let mut tlv = Vec::new();
        let mut push = |kind: u8, value: &[u8]| {
            let len = u8::try_from(value.len()).expect("TLV values fit in 255 bytes");
            tlv.extend([kind, len]);
            tlv.extend_from_slice(value);
        };
        push(TLV_SECRET, self.secret.as_secret_bytes());
        push(TLV_ID, self.id.as_bytes());
        push(TLV_EPOCH, &self.epoch.to_be_bytes());
        for relay in &self.relays {
            let url = relay.as_str_without_trailing_slash();
            if url.len() <= usize::from(u8::MAX) {
                push(TLV_RELAY, url.as_bytes());
            }
        }
        bech32::encode::<Bech32>(RECOVERY_HRP, &tlv).expect("a recovery key is short")
    }

    pub fn decode(text: &str) -> Result<Self> {
        let malformed = || Error::Malformed("invalid recovery key");
        let (hrp, tlv) = bech32::decode(text.trim()).map_err(|_| malformed())?;
        if hrp != RECOVERY_HRP {
            return Err(malformed());
        }
        let (mut secret, mut id, mut epoch, mut relays) = (None, None, None, Vec::new());
        let mut rest = tlv.as_slice();
        while let [kind, len, tail @ ..] = rest {
            let len = usize::from(*len);
            if tail.len() < len {
                return Err(malformed());
            }
            let (value, next) = tail.split_at(len);
            rest = next;
            match *kind {
                TLV_SECRET => secret = Some(SecretKey::from_slice(value).map_err(|_| malformed())?),
                TLV_ID => id = Some(DriveId(value.try_into().map_err(|_| malformed())?)),
                TLV_EPOCH => {
                    epoch = Some(u32::from_be_bytes(
                        value.try_into().map_err(|_| malformed())?,
                    ));
                }
                TLV_RELAY => {
                    let url = std::str::from_utf8(value).map_err(|_| malformed())?;
                    relays.push(RelayUrl::parse(url).map_err(|_| malformed())?);
                }
                _ => {}
            }
        }
        if !rest.is_empty() {
            return Err(malformed());
        }
        Ok(Self {
            secret: secret.ok_or_else(malformed)?,
            id: id.ok_or_else(malformed)?,
            epoch: epoch.ok_or_else(malformed)?,
            relays,
        })
    }

    pub fn keys(&self) -> DriveKeys {
        DriveKeys::from_secret(self.secret.clone())
    }
}

impl fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryKey")
            .field("id", &self.id)
            .field("epoch", &self.epoch)
            .field("relays", &self.relays)
            .finish_non_exhaustive()
    }
}

fn nostr_error(e: impl fmt::Display) -> Error {
    Error::Nostr(e.to_string())
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    #[tokio::test]
    async fn round_trip_through_the_users_own_key() {
        let user = Keys::generate();
        let descriptor = Descriptor::generate("Lupine");
        let event = descriptor.seal(&user).await.unwrap();
        assert_eq!(event.kind, DESCRIPTOR_KIND);
        assert!(event.tags.is_empty());
        assert!(!event.content.contains(&descriptor.secret.to_secret_hex()));

        let opened = Descriptor::open(&user, &event).await.unwrap().unwrap();
        assert_eq!((opened.id, opened.epoch), (descriptor.id, 0));
        assert_eq!(opened.name, "Lupine");
        assert_eq!(opened.keys().public_key(), descriptor.keys().public_key());

        let stranger = Keys::generate();
        assert!(Descriptor::open(&stranger, &event).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_highest_epoch_then_the_latest_wins() {
        let user = Keys::generate();
        let first = Descriptor::generate("Lupine");
        let mut renamed = first.clone();
        renamed.name = "Renamed".into();
        let mut rotated = first.clone();
        rotated.epoch = 1;
        rotated.secret = SecretKey::generate();

        let at = |descriptor: &Descriptor, secs: u64| {
            let descriptor = descriptor.clone();
            let user = user.clone();
            async move {
                let json = serde_json::to_string(&Json {
                    v: FORMAT_VERSION,
                    id: descriptor.id,
                    epoch: descriptor.epoch,
                    secret: descriptor.secret.to_secret_hex(),
                    name: descriptor.name.clone(),
                })
                .unwrap();
                let content = user
                    .nip44_encrypt_async(&user.public_key(), &json)
                    .await
                    .unwrap();
                let event = EventBuilder::new(DESCRIPTOR_KIND, content)
                    .custom_created_at(Timestamp::from_secs(secs))
                    .finalize_async(&user)
                    .await
                    .unwrap();
                FoundDrive { event, descriptor }
            }
        };
        let other = at(&Descriptor::generate("Other"), 1).await;
        let found = vec![
            at(&rotated, 10).await,
            at(&renamed, 30).await,
            at(&first, 20).await,
            other.clone(),
        ];
        let (current, superseded) = super::current(found.clone());
        let names: Vec<_> = current.iter().map(|d| d.descriptor.name.as_str()).collect();
        assert_eq!(names, ["Lupine", "Other"]);
        assert_eq!(current[0].descriptor.epoch, 1);
        assert_eq!(superseded.len(), 2);

        let (current, _) = super::current(found[1..].to_vec());
        let lupine = current
            .iter()
            .find(|d| d.descriptor.id == first.id)
            .unwrap();
        assert_eq!(lupine.descriptor.name, "Renamed");
    }

    #[test]
    fn recovery_keys_round_trip() {
        let descriptor = Descriptor::generate("Lupine");
        let relays = [
            RelayUrl::parse("wss://relay.example.com").unwrap(),
            RelayUrl::parse("wss://other.example.org/inbox").unwrap(),
        ];
        let key = descriptor.recovery_key(&relays);
        let text = key.encode();
        assert!(text.starts_with("lupine1"));

        let decoded = RecoveryKey::decode(&text).unwrap();
        assert_eq!(decoded.secret, descriptor.secret);
        assert_eq!((decoded.id, decoded.epoch), (descriptor.id, 0));
        assert_eq!(decoded.relays, relays);

        let mut corrupted = text.clone().into_bytes();
        let last = corrupted.len() - 1;
        corrupted[last] = if corrupted[last] == b'q' { b'p' } else { b'q' };
        assert!(RecoveryKey::decode(std::str::from_utf8(&corrupted).unwrap()).is_err());
        let npub = bech32::encode::<Bech32>(Hrp::parse("npub").unwrap(), &[0; 32]).unwrap();
        assert!(RecoveryKey::decode(&npub).is_err());
    }
}
