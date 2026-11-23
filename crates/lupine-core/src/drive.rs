//! Drive keys, and node versions published as events that look like NIP-59
//! gift wraps: kind 1059, a fresh signing key, a randomized `created_at`
//! and a NIP-44 v2 payload. There is no seal inside. The payload is
//! encrypted with a key only the drive holds, which also authenticates it,
//! since anyone can publish a gift wrap to the drive's pubkey.

use nostr::event::FinalizeEvent;
use nostr::event::{Event, EventBuilder, Kind, Tag};
use nostr::key::{Keys, PublicKey, SecretKey};
use nostr::nips::nip44::v2::{ConversationKey, decrypt_to_bytes, encrypt_to_bytes_with_nonce};
use nostr::types::Timestamp;

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
        let key = self.wrap_key(&signer.public_key());
        let payload = encrypt_to_bytes_with_nonce(&key, &plaintext, random_bytes())
            .map_err(|e| Error::Nostr(e.to_string()))?;
        let backdate =
            getrandom::u64().expect("the OS random number generator failed") % MAX_BACKDATE_SECS;
        let event = EventBuilder::new(Kind::GiftWrap, encoding::encode(&payload))
            .tag(Tag::public_key(self.public_key()))
            .custom_created_at(Timestamp::from_secs(
                Timestamp::now().as_secs().saturating_sub(backdate),
            ))
            .finalize(signer)
            .map_err(|e| Error::Nostr(e.to_string()))?;
        Ok((VersionId::of(&plaintext), event))
    }

    /// `None` for gift wraps that are not versions of this drive: messages,
    /// versions of another drive, forgeries, payloads moved to another wrap.
    pub fn unwrap(&self, event: &Event) -> Result<Option<(VersionId, NodeVersion)>> {
        let addressed_here = event.tags.public_keys().any(|pk| pk == self.public_key());
        if event.kind != Kind::GiftWrap || !addressed_here || event.verify().is_err() {
            return Ok(None);
        }
        let Some(payload) = encoding::decode(&event.content) else {
            return Ok(None);
        };
        let Ok(plaintext) = decrypt_to_bytes(&self.wrap_key(&event.pubkey), &payload) else {
            return Ok(None);
        };
        let mut version: NodeVersion = serde_json::from_slice(&plaintext)?;
        version.normalize();
        version.validate()?;
        Ok(Some((VersionId::of(&plaintext), version)))
    }
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
        let (unwrapped_id, unwrapped) = drive.unwrap(&event).unwrap().unwrap();
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

        let (_, unwrapped) = drive.unwrap(&tombstone).unwrap().unwrap();
        let json = serde_json::to_vec(&unwrapped).unwrap();
        let mut padded = json.clone();
        padded.resize(MIN_PLAINTEXT_LEN, b' ');
        assert_eq!(id, VersionId::of(&padded), "the id covers the padding");
        assert_ne!(id, VersionId::of(&json));
    }

    #[test]
    fn other_drives_ignore_it() {
        let (_, event) = DriveKeys::generate().wrap(&folder("Photos")).unwrap();
        assert!(DriveKeys::generate().unwrap(&event).unwrap().is_none());
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
        assert!(drive.unwrap(&forged).unwrap().is_none());

        let dm = PrivateDirectMessageBuilder::new(drive.public_key(), "hello")
            .finalize(&attacker)
            .unwrap();
        assert!(drive.unwrap(&dm).unwrap().is_none());
    }

    #[test]
    fn payloads_copied_into_another_wrap_do_not_decrypt() {
        let drive = DriveKeys::generate();
        let (_, event) = drive.wrap(&folder("Photos")).unwrap();
        let copy = rewrap(&event, event.content.clone(), &Keys::generate());
        copy.verify().unwrap();
        assert!(drive.unwrap(&copy).unwrap().is_none());
    }

    #[test]
    fn tampered_payloads_are_ignored() {
        let drive = DriveKeys::generate();
        let signer = Keys::generate();
        let (_, event) = drive.wrap_with(&folder("Photos"), &signer).unwrap();
        let mut payload = encoding::decode(&event.content).unwrap();
        payload[40] ^= 1;

        let resigned = rewrap(&event, encoding::encode(&payload), &signer);
        assert!(drive.unwrap(&resigned).unwrap().is_none(), "NIP-44 MAC");

        let mut unsigned = event.clone();
        unsigned.content = encoding::encode(&payload);
        assert!(drive.unwrap(&unsigned).unwrap().is_none(), "signature");
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
}
