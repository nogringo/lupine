//! Drive keys, and node versions published as events that look like NIP-59
//! gift wraps: kind 1059, a fresh signing key, a randomized `created_at`
//! and a NIP-44 v2 payload. There is no seal inside. The payload is
//! encrypted with a key only the drive holds, which also authenticates it,
//! since anyone can publish a gift wrap to the drive's pubkey.

use nostr::event::{Event, EventBuilder, EventId, Kind, Tag};
use nostr::event::{FinalizeEvent, IntoEventBuilder};
use nostr::key::{Keys, PublicKey, SecretKey};
use nostr::nips::nip09::EventDeletionRequest;
use nostr::nips::nip17::{self, InboxRelayList};
use nostr::nips::nip44::v2::{ConversationKey, decrypt_to_bytes, encrypt_to_bytes_with_nonce};
use nostr::nips::nip62::{VanishRequest, VanishTarget};
use nostr::types::{RelayUrl, Timestamp};

use crate::crypto::{SymmetricKey, random_bytes};
use crate::encoding;
use crate::error::{Error, Result};
use crate::ids::VersionId;
use crate::node::NodeVersion;

/// NIP-17 randomizes gift wrap timestamps up to two days in the past.
const MAX_BACKDATE_SECS: u64 = 2 * 24 * 60 * 60;
/// What NIP-44 pads the seal of the shortest NIP-17 message to, so no
/// version is smaller than a one-word DM.
pub const MIN_PLAINTEXT_LEN: usize = 896;
/// Wraps named by one deletion, to stay under relay size limits.
pub const DELETION_BATCH: usize = 100;

/// What a gift wrap addressed to the drive holds.
#[derive(Debug)]
pub enum Unwrapped {
    Version(VersionId, Box<NodeVersion>),
    /// Decrypts with the drive keys but is not a version this client can
    /// read, such as a later format. Kept, and ignored.
    Unreadable(Error),
    /// Fails to decrypt: a message, a forgery, a payload moved from another
    /// wrap. The drive deletes it.
    Junk,
}

pub struct DriveKeys {
    keys: Keys,
    metadata: SymmetricKey,
}

impl DriveKeys {
    pub fn generate() -> Self {
        Self::new(Keys::generate())
    }

    pub fn from_secret(secret: SecretKey) -> Self {
        Self::new(Keys::new(secret))
    }

    fn new(keys: Keys) -> Self {
        let secret = SymmetricKey::from_bytes(keys.secret_key().to_secret_bytes());
        Self {
            metadata: secret.derive(b"lupine/v1/drive/metadata"),
            keys,
        }
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    /// Signs Blossom auth tokens and deletions. Versions are signed by
    /// throwaway keys instead.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Depends on the wrap's signer, so a payload copied into another wrap
    /// no longer decrypts.
    fn wrap_key(&self, signer: &PublicKey) -> ConversationKey {
        let info = [&b"lupine/v1/wrap"[..], signer.as_bytes()].concat();
        ConversationKey::new(*self.metadata.derive(&info).as_bytes())
    }

    pub fn wrap(&self, version: &NodeVersion) -> Result<(VersionId, Event)> {
        self.wrap_with(version, &Keys::generate())
    }

    fn wrap_with(&self, version: &NodeVersion, signer: &Keys) -> Result<(VersionId, Event)> {
        version.validate()?;
        let mut plaintext = serde_json::to_vec(version)?;
        if plaintext.len() < MIN_PLAINTEXT_LEN {
            plaintext.resize(MIN_PLAINTEXT_LEN, b' ');
        }
        let event = self.seal(&plaintext, signer)?;
        Ok((VersionId::of(&plaintext), event))
    }

    /// `event`, a wrap addressed to `from`, wrapped again for this drive
    /// around the same plaintext, padding included: it carries the same
    /// version, under the same id. `None` unless `from` can read the wrap.
    pub fn rewrap(&self, from: &DriveKeys, event: &Event) -> Option<(VersionId, Event)> {
        let plaintext = from.plaintext(event)?;
        let rewrapped = self.seal(&plaintext, &Keys::generate()).ok()?;
        Some((VersionId::of(&plaintext), rewrapped))
    }

    fn seal(&self, plaintext: &[u8], signer: &Keys) -> Result<Event> {
        let key = self.wrap_key(&signer.public_key());
        let payload = encrypt_to_bytes_with_nonce(&key, plaintext, random_bytes())
            .map_err(|e| Error::Nostr(e.to_string()))?;
        let backdate =
            getrandom::u64().expect("the OS random number generator failed") % MAX_BACKDATE_SECS;
        EventBuilder::new(Kind::GiftWrap, encoding::encode(&payload))
            .tag(Tag::public_key(self.public_key()))
            .custom_created_at(Timestamp::from_secs(
                Timestamp::now().as_secs().saturating_sub(backdate),
            ))
            .finalize(signer)
            .map_err(|e| Error::Nostr(e.to_string()))
    }

    fn addressed_here(&self, event: &Event) -> bool {
        let addressed = event.tags.public_keys().any(|pk| pk == self.public_key());
        event.kind == Kind::GiftWrap && addressed && event.verify().is_ok()
    }

    /// What a wrap addressed to the drive holds, padding included. `None`
    /// for junk.
    pub(crate) fn plaintext(&self, event: &Event) -> Option<Vec<u8>> {
        if !self.addressed_here(event) {
            return None;
        }
        let key = self.wrap_key(&event.pubkey);
        encoding::decode(&event.content).and_then(|payload| decrypt_to_bytes(&key, &payload).ok())
    }

    /// `None` unless the event is a validly signed gift wrap addressed to
    /// the drive.
    pub fn unwrap(&self, event: &Event) -> Option<Unwrapped> {
        if !self.addressed_here(event) {
            return None;
        }
        let Some(plaintext) = self.plaintext(event) else {
            return Some(Unwrapped::Junk);
        };
        let version = serde_json::from_slice::<NodeVersion>(&plaintext)
            .map_err(Error::from)
            .and_then(|mut version| {
                version.normalize();
                version.validate()?;
                Ok(version)
            });
        Some(match version {
            Ok(version) => Unwrapped::Version(VersionId::of(&plaintext), Box::new(version)),
            Err(e) => Unwrapped::Unreadable(e),
        })
    }

    /// NIP-09 deletions of wraps addressed to the drive. NIP-59 has relays
    /// honor them from the recipient, whatever key signed the wrap.
    pub fn deletions(&self, wraps: impl IntoIterator<Item = EventId>) -> Result<Vec<Event>> {
        let wraps: Vec<EventId> = wraps.into_iter().collect();
        wraps
            .chunks(DELETION_BATCH)
            .map(|batch| {
                let kind = Tag::parse(["k", "1059"]).map_err(|e| Error::Nostr(e.to_string()))?;
                EventDeletionRequest::new()
                    .ids(batch.iter().copied())
                    .into_event_builder()
                    .tag(kind)
                    .finalize(&self.keys)
                    .map_err(|e| Error::Nostr(e.to_string()))
            })
            .collect()
    }

    /// NIP-62 request to vanish: the key is retired after a rotation, and
    /// the relays that honor it delete the wraps addressed to it.
    pub fn vanish(&self, relays: impl IntoIterator<Item = RelayUrl>) -> Result<Event> {
        VanishRequest::new(VanishTarget::relays(relays))
            .into_event_builder()
            .finalize(&self.keys)
            .map_err(|e| Error::Nostr(e.to_string()))
    }

    /// Kind 10050: the relays that store the drive's versions.
    pub fn relay_list(&self, relays: impl IntoIterator<Item = RelayUrl>) -> Result<Event> {
        InboxRelayList::new(relays)
            .into_event_builder()
            .finalize(&self.keys)
            .map_err(|e| Error::Nostr(e.to_string()))
    }

    /// Kind 10063 (BUD-03): the drive's Blossom servers, most trusted first.
    pub fn server_list<I, S>(&self, servers: I) -> Result<Event>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let tags = servers
            .into_iter()
            .map(|url| Tag::parse(["server", url.as_ref()]))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Nostr(e.to_string()))?;
        EventBuilder::new(Kind::BlossomServerList, "")
            .tags(tags)
            .finalize(&self.keys)
            .map_err(|e| Error::Nostr(e.to_string()))
    }
}

/// `event` is a request to vanish from the drive key `drive`, which a key
/// rotation retired.
pub fn vanished(drive: &PublicKey, event: &Event) -> bool {
    event.kind == Kind::RequestToVanish && event.pubkey == *drive && event.verify().is_ok()
}

/// The wraps a drive deletion names, or nothing when `event` is not one.
pub fn deleted_wraps(drive: &PublicKey, event: &Event) -> Vec<EventId> {
    if event.kind != Kind::EventDeletion || event.pubkey != *drive || event.verify().is_err() {
        return Vec::new();
    }
    event.tags.event_ids().collect()
}

pub fn read_relay_list(event: &Event) -> Vec<RelayUrl> {
    if event.kind != Kind::InboxRelays {
        return Vec::new();
    }
    nip17::extract_relay_list(event).collect()
}

pub fn read_server_list(event: &Event) -> Vec<String> {
    if event.kind != Kind::BlossomServerList {
        return Vec::new();
    }
    event
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [name, url, ..] if name == "server" => Some(url.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use nostr::nips::nip17::PrivateDirectMessageBuilder;
    use nostr::nips::nip44;

    use super::*;
    use crate::content::{ContentDescriptor, FileKey, INLINE_CHUNK_LIMIT};
    use crate::ids::{DeviceId, NodeId};
    use crate::node::{NodeType, Stamp};

    fn stamp() -> Stamp {
        Stamp::next(DeviceId::random(), 0, 0)
    }

    fn folder(name: &str) -> NodeVersion {
        NodeVersion::folder(stamp(), NodeId::random(), None, name)
    }

    fn read(drive: &DriveKeys, event: &Event) -> Option<(VersionId, NodeVersion)> {
        match drive.unwrap(event)? {
            Unwrapped::Version(id, version) => Some((id, *version)),
            _ => None,
        }
    }

    fn rewrap(event: &Event, content: String, signer: &Keys) -> Event {
        EventBuilder::new(Kind::GiftWrap, content)
            .tags(event.tags.clone())
            .custom_created_at(event.created_at)
            .finalize(signer)
            .unwrap()
    }

    #[test]
    fn round_trip() {
        let drive = DriveKeys::generate();
        let node = NodeId::random();
        let first = NodeVersion::folder(stamp(), node, None, "draft");
        let version = NodeVersion::file(
            stamp(),
            node,
            Some(NodeId::random()),
            "café.txt",
            &FileKey::generate(),
            ContentDescriptor {
                size: 5,
                chunk_size: 1 << 16,
                chunks: Some(vec![7; 64]),
                table: None,
            },
        )
        .replacing(&[&first])
        .unwrap();

        let (id, event) = drive.wrap(&version).unwrap();
        let (unwrapped_id, unwrapped) = read(&drive, &event).unwrap();
        assert_eq!(unwrapped_id, id);
        assert_eq!(unwrapped.node_type, NodeType::File);
        assert_eq!(
            serde_json::to_value(&unwrapped).unwrap(),
            serde_json::to_value(&version).unwrap()
        );
    }

    #[test]
    fn looks_like_a_gift_wrap() {
        let drive = DriveKeys::generate();
        let (_, first) = drive.wrap(&folder("Photos")).unwrap();
        let (_, second) = drive.wrap(&folder("Photos")).unwrap();

        assert_eq!(first.kind, Kind::GiftWrap);
        assert_eq!(
            first.tags.clone().to_vec(),
            vec![Tag::public_key(drive.public_key())]
        );
        assert_ne!(first.pubkey, drive.public_key());
        assert_ne!(first.pubkey, second.pubkey);
        first.verify().unwrap();

        let now = Timestamp::now().as_secs();
        let created_at = first.created_at.as_secs();
        assert!(created_at <= now && created_at + MAX_BACKDATE_SECS >= now);

        let payload = encoding::decode(&first.content).unwrap();
        assert_eq!(payload[0], 2, "NIP-44 v2 version byte");
    }

    #[test]
    fn no_version_is_smaller_than_a_one_word_dm() {
        let drive = DriveKeys::generate();
        let dm = PrivateDirectMessageBuilder::new(drive.public_key(), "hi")
            .finalize(&Keys::generate())
            .unwrap();
        let (id, tombstone) = drive
            .wrap(&NodeVersion::tombstone(stamp(), &[&folder("a")]).unwrap())
            .unwrap();
        assert_eq!(tombstone.content.len(), dm.content.len());

        let (_, unwrapped) = read(&drive, &tombstone).unwrap();
        let json = serde_json::to_vec(&unwrapped).unwrap();
        let mut padded = json.clone();
        padded.resize(MIN_PLAINTEXT_LEN, b' ');
        assert_eq!(id, VersionId::of(&padded), "the id covers the padding");
        assert_ne!(id, VersionId::of(&json));
    }

    #[test]
    fn a_wrap_moves_to_a_new_key_with_its_version() {
        let (old, new) = (DriveKeys::generate(), DriveKeys::generate());
        let version = folder("Photos");
        let (id, event) = old.wrap(&version).unwrap();
        let (moved_id, moved) = new.rewrap(&old, &event).unwrap();
        assert_eq!(moved_id, id);
        assert_ne!(moved.pubkey, event.pubkey);
        assert!(moved.tags.public_keys().eq([new.public_key()]));
        let (read_id, read_version) = read(&new, &moved).unwrap();
        assert_eq!((read_id, read_version.name), (id, version.name));
        assert!(old.unwrap(&moved).is_none());
        assert!(
            new.rewrap(&new, &event).is_none(),
            "only the old key reads it"
        );
    }

    #[test]
    fn other_drives_ignore_it() {
        let (_, event) = DriveKeys::generate().wrap(&folder("Photos")).unwrap();
        assert!(DriveKeys::generate().unwrap(&event).is_none());
    }

    #[test]
    fn real_gift_wraps_to_the_drive_cannot_inject_versions() {
        let drive = DriveKeys::generate();
        let attacker = Keys::generate();
        let json = serde_json::to_vec(&folder("Injected")).unwrap();
        let content = nip44::encrypt(
            attacker.secret_key(),
            &drive.public_key(),
            &json,
            nip44::Version::V2,
        )
        .unwrap();
        let forged = EventBuilder::new(Kind::GiftWrap, content)
            .tag(Tag::public_key(drive.public_key()))
            .finalize(&attacker)
            .unwrap();
        assert!(matches!(drive.unwrap(&forged), Some(Unwrapped::Junk)));

        let dm = PrivateDirectMessageBuilder::new(drive.public_key(), "hello")
            .finalize(&attacker)
            .unwrap();
        assert!(matches!(drive.unwrap(&dm), Some(Unwrapped::Junk)));
    }

    #[test]
    fn payloads_copied_into_another_wrap_do_not_decrypt() {
        let drive = DriveKeys::generate();
        let (_, event) = drive.wrap(&folder("Photos")).unwrap();
        let copy = rewrap(&event, event.content.clone(), &Keys::generate());
        copy.verify().unwrap();
        assert!(matches!(drive.unwrap(&copy), Some(Unwrapped::Junk)));
    }

    #[test]
    fn tampered_payloads_are_ignored() {
        let drive = DriveKeys::generate();
        let signer = Keys::generate();
        let (_, event) = drive.wrap_with(&folder("Photos"), &signer).unwrap();
        let mut payload = encoding::decode(&event.content).unwrap();
        payload[40] ^= 1;

        let resigned = rewrap(&event, encoding::encode(&payload), &signer);
        assert!(
            matches!(drive.unwrap(&resigned), Some(Unwrapped::Junk)),
            "NIP-44 MAC"
        );

        let mut unsigned = event.clone();
        unsigned.content = encoding::encode(&payload);
        assert!(drive.unwrap(&unsigned).is_none(), "signature");
    }

    #[test]
    fn large_inline_tables_stay_small() {
        let version = NodeVersion::file(
            stamp(),
            NodeId::random(),
            None,
            "big.bin",
            &FileKey::generate(),
            ContentDescriptor {
                size: 1 << 29,
                chunk_size: 8 << 20,
                chunks: Some(vec![0xab; INLINE_CHUNK_LIMIT * 64]),
                table: None,
            },
        );
        let (_, event) = DriveKeys::generate().wrap(&version).unwrap();
        assert!(
            event.as_json().len() < 16 * 1024,
            "{} bytes",
            event.as_json().len()
        );
    }

    #[test]
    fn later_formats_are_kept_but_not_read() {
        let drive = DriveKeys::generate();
        let mut later = serde_json::to_value(folder("Photos")).unwrap();
        later["v"] = 2.into();
        let mut plaintext = serde_json::to_vec(&later).unwrap();
        plaintext.resize(MIN_PLAINTEXT_LEN, b' ');
        let signer = Keys::generate();
        let payload = encrypt_to_bytes_with_nonce(
            &drive.wrap_key(&signer.public_key()),
            &plaintext,
            random_bytes(),
        )
        .unwrap();
        let event = EventBuilder::new(Kind::GiftWrap, encoding::encode(&payload))
            .tag(Tag::public_key(drive.public_key()))
            .finalize(&signer)
            .unwrap();
        assert!(matches!(
            drive.unwrap(&event),
            Some(Unwrapped::Unreadable(Error::UnsupportedVersion(2)))
        ));
    }

    #[test]
    fn deletions_name_wraps_in_batches() {
        let drive = DriveKeys::generate();
        let wraps: Vec<EventId> = (0..DELETION_BATCH + 1)
            .map(|_| EventId::from_byte_array(random_bytes()))
            .collect();
        let deletions = drive.deletions(wraps.iter().copied()).unwrap();
        assert_eq!(deletions.len(), 2);
        let first = &deletions[0];
        assert_eq!(first.kind, Kind::EventDeletion);
        assert_eq!(first.pubkey, drive.public_key());
        assert!(first.tags.iter().any(|t| t.as_slice() == ["k", "1059"]));
        let named: Vec<EventId> = deletions
            .iter()
            .flat_map(|d| deleted_wraps(&drive.public_key(), d))
            .collect();
        assert_eq!(named, wraps);
        assert!(deleted_wraps(&Keys::generate().public_key(), first).is_empty());
    }

    #[test]
    fn lists_round_trip() {
        let drive = DriveKeys::generate();
        let relays = vec![RelayUrl::parse("wss://relay.example.com").unwrap()];
        let event = drive.relay_list(relays.clone()).unwrap();
        assert_eq!(event.kind, Kind::InboxRelays);
        assert_eq!(read_relay_list(&event), relays);

        let servers = ["https://a.example.com", "https://b.example.com"];
        let event = drive.server_list(servers).unwrap();
        assert_eq!(event.kind, Kind::BlossomServerList);
        assert_eq!(read_server_list(&event), servers);
        assert!(read_relay_list(&event).is_empty());
    }
}
