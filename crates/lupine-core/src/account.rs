//! The user's side of a drive, on the user's write relays (NIP-65): the
//! descriptors that hold drive secrets, and the drives' lists of relays and
//! Blossom servers, which a new device needs before it can open a drive.

use std::cmp::Reverse;
use std::sync::Arc;

use futures::future::try_join_all;
use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::nips::nip65::{self, RelayMetadata};
use nostr::types::RelayUrl;

use crate::blossom::BlossomStore;
use crate::content::{ChunkLayout, DEFAULT_CHUNK_SIZE};
use crate::descriptor::{self, DESCRIPTOR_KIND, Descriptor, FoundDrive, UserSigner};
use crate::drive::{DriveKeys, read_relay_list, read_server_list};
use crate::error::{Error, Result};
use crate::relays::{Relays, Sent};
use crate::store::Store;

/// Where kind 10002 lists are looked up.
pub const INDEX_RELAYS: [&str; 4] = [
    "wss://purplepag.es",
    "wss://indexer.coracle.social",
    "wss://relay.primal.net",
    "wss://relay.ditto.pub",
];
/// The user's relays when they publish no NIP-65 list, so that every
/// device looks for descriptors in the same place.
pub const FALLBACK_RELAYS: [&str; 2] = ["wss://relay.primal.net", "wss://relay.ditto.pub"];

/// How a device finds the user's write relays.
#[derive(Clone, Debug)]
pub struct Discovery {
    pub index: Vec<RelayUrl>,
    pub fallback: Vec<RelayUrl>,
}

impl Default for Discovery {
    fn default() -> Self {
        let parse = |urls: &[&str]| {
            urls.iter()
                .map(|url| RelayUrl::parse(url).expect("a valid relay URL"))
                .collect()
        };
        Self {
            index: parse(&INDEX_RELAYS),
            fallback: parse(&FALLBACK_RELAYS),
        }
    }
}

/// The relays the latest kind 10002 of `user` marks `write` or leaves
/// unmarked. Empty when the user publishes none.
pub async fn write_relays(user: PublicKey, index: &[RelayUrl]) -> Result<Vec<RelayUrl>> {
    let relays = Relays::public(index.iter().cloned()).await?;
    let found = relays
        .query(&Filter::new().kind(Kind::RelayList).author(user))
        .await;
    relays.shutdown().await;
    let latest = found?
        .into_iter()
        .filter(|e| e.pubkey == user && e.verify().is_ok())
        .max_by_key(|e| (e.created_at, Reverse(e.id)));
    let Some(latest) = latest else {
        return Ok(Vec::new());
    };
    Ok(nip65::extract_relay_list(&latest)
        .filter(|(_, marker)| *marker != Some(RelayMetadata::Read))
        .map(|(url, _)| url)
        .collect())
}

/// Where a drive keeps its versions and its blobs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriveLists {
    pub relays: Vec<RelayUrl>,
    /// Most trusted first.
    pub servers: Vec<String>,
}

impl DriveLists {
    /// The latest kinds 10050 and 10063 of `drive` on `relays`: the user's
    /// relays, or the drive's own.
    pub async fn find(relays: &Relays, drive: PublicKey) -> Result<Self> {
        let filter = Filter::new()
            .kinds([Kind::InboxRelays, Kind::BlossomServerList])
            .author(drive);
        let events = relays.query(&filter).await?;
        let latest = |kind: Kind| {
            events
                .iter()
                .filter(|e| e.kind == kind && e.pubkey == drive && e.verify().is_ok())
                .max_by_key(|e| (e.created_at, Reverse(e.id)))
        };
        Ok(Self {
            relays: latest(Kind::InboxRelays)
                .map(read_relay_list)
                .unwrap_or_default(),
            servers: latest(Kind::BlossomServerList)
                .map(read_server_list)
                .unwrap_or_default(),
        })
    }
}

pub struct Account<S> {
    signer: Arc<S>,
    user: PublicKey,
    relays: Relays,
}

impl<S: UserSigner> Account<S> {
    /// `relays` are the user's write relays.
    pub async fn connect(
        signer: Arc<S>,
        relays: impl IntoIterator<Item = RelayUrl>,
    ) -> Result<Self> {
        let user = signer
            .get_public_key_async()
            .await
            .map_err(|e| Error::Nostr(e.to_string()))?;
        let relays = Relays::for_user(Arc::clone(&signer), relays).await?;
        Ok(Self {
            signer,
            user,
            relays,
        })
    }

    /// Connects to the user's write relays, or to the fallback relays when
    /// the user lists none.
    pub async fn sign_in(signer: Arc<S>, discovery: &Discovery) -> Result<Self> {
        let user = signer
            .get_public_key_async()
            .await
            .map_err(|e| Error::Nostr(e.to_string()))?;
        let mut relays = write_relays(user, &discovery.index).await?;
        if relays.is_empty() {
            relays = discovery.fallback.clone();
        }
        Self::connect(signer, relays).await
    }

    pub fn public_key(&self) -> PublicKey {
        self.user
    }

    pub fn signer(&self) -> &Arc<S> {
        &self.signer
    }

    pub fn relays(&self) -> &Relays {
        &self.relays
    }

    /// The current descriptor of each of the user's drives.
    pub async fn drives(&self) -> Result<Vec<FoundDrive>> {
        let filter = Filter::new().kind(DESCRIPTOR_KIND).author(self.user);
        let mut found = Vec::new();
        for event in self.relays.query(&filter).await? {
            if let Some(descriptor) = Descriptor::open(&*self.signer, &event).await? {
                found.push(FoundDrive { event, descriptor });
            }
        }
        Ok(descriptor::current(found).0)
    }

    /// The user's own inbox relays (kind 10050) and Blossom servers (kind
    /// 10063), to suggest for a new drive.
    pub async fn suggestions(&self) -> Result<DriveLists> {
        DriveLists::find(&self.relays, self.user).await
    }

    /// Creates a drive and announces it: the descriptor to the user's
    /// relays and the drive relays, and the drive's lists to both. Fails
    /// before announcing anything when a server would refuse encrypted
    /// chunks.
    pub async fn create_drive(
        &self,
        name: &str,
        drive_relays: &[RelayUrl],
        servers: &[String],
    ) -> Result<(FoundDrive, DriveLists)> {
        if drive_relays.is_empty() || servers.is_empty() {
            return Err(Error::Malformed(
                "a drive needs a relay and a Blossom server",
            ));
        }
        let descriptor = Descriptor::generate(name);
        let keys = descriptor.keys();
        check_servers(servers, &keys).await?;
        let event = descriptor.seal(&*self.signer).await?;
        let lists = DriveLists {
            relays: drive_relays.to_vec(),
            servers: servers.to_vec(),
        };
        let announcements = [
            event.clone(),
            keys.relay_list(lists.relays.iter().cloned())?,
            keys.server_list(&lists.servers)?,
        ];
        let drive = Relays::for_drive(&keys, drive_relays.iter().cloned()).await?;
        for announcement in &announcements {
            let sent = self.relays.send_all(announcement).await;
            let on_drive = drive.send_all(announcement).await;
            if sent.accepted.is_empty() && on_drive.accepted.is_empty() {
                drive.shutdown().await;
                return Err(Error::NotPublished(sent.reasons()));
            }
        }
        drive.shutdown().await;
        Ok((FoundDrive { event, descriptor }, lists))
    }

    /// The latest kinds 10050 and 10063 of `drive` on the user's relays.
    pub async fn lists(&self, drive: PublicKey) -> Result<DriveLists> {
        DriveLists::find(&self.relays, drive).await
    }

    /// What a new device needs to open `found`, which every device
    /// announces on the user's relays. Republishes the descriptor to those
    /// that lack it.
    pub async fn open(&self, found: &FoundDrive) -> Result<DriveLists> {
        let lists = self.lists(found.descriptor.keys().public_key()).await?;
        if lists.relays.is_empty() || lists.servers.is_empty() {
            return Err(Error::Malformed(
                "the drive's relays and servers are not on the user's relays",
            ));
        }
        self.keep(&found.event).await;
        Ok(lists)
    }

    /// Republishes `event`, unchanged, to the user's relays that lack it.
    /// Every device does this for the descriptor it read, so it is lost only
    /// if every relay and every device loses it.
    pub async fn keep(&self, event: &Event) -> Sent {
        let lacking = self.relays.lacking(event).await;
        self.relays.send(event, &lacking).await
    }

    pub async fn shutdown(&self) {
        self.relays.shutdown().await;
    }
}

/// Fails with the first server that would refuse an encrypted chunk.
async fn check_servers(servers: &[String], keys: &DriveKeys) -> Result<()> {
    let blobs = BlossomStore::new(servers, keys.keys())?;
    let chunk = DEFAULT_CHUNK_SIZE;
    let len = ChunkLayout::new(chunk.into(), chunk)?.blob_len(0) as u64;
    try_join_all(blobs.servers().iter().map(|s| s.check_upload(len))).await?;
    Ok(())
}

/// Republishes the descriptor this device read, unchanged, to the user's
/// relays and the drive relays that lack it. Needs no user key: relays that
/// ask for AUTH before taking it are left out. Returns how many took it.
pub async fn keep_descriptor(store: &Store, keys: &DriveKeys) -> Result<usize> {
    let (Some(kept), Some(drive)) = (store.descriptor()?, store.drive()?) else {
        return Ok(0);
    };
    let mut accepted = 0;
    let user = Relays::public(kept.user_relays).await;
    let drive = Relays::for_drive(keys, drive.relays).await;
    for relays in [user, drive].into_iter().flatten() {
        let lacking = relays.lacking(&kept.event).await;
        accepted += relays.send(&kept.event, &lacking).await.accepted.len();
        relays.shutdown().await;
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use nostr::event::{FinalizeEvent, IntoEventBuilder};
    use nostr::key::Keys;
    use nostr::nips::nip65::RelayList;
    use nostr_connect::prelude::{
        NostrConnectKeys, NostrConnectRemoteSigner, NostrConnectRequest, NostrConnectSignerActions,
    };

    use super::*;
    use crate::identity::UserKey;
    use crate::store::{DescriptorRecord, DriveRecord};
    use crate::testing::{TestRelay, blossom_server};

    fn relay_list(user: &Keys, relays: &[(&RelayUrl, Option<RelayMetadata>)]) -> Event {
        let relays = relays.iter().map(|(url, marker)| ((*url).clone(), *marker));
        RelayList::new(relays)
            .into_event_builder()
            .finalize(user)
            .unwrap()
    }

    #[tokio::test]
    async fn sign_in_uses_the_users_write_relays() {
        let (index, write, read) = (
            TestRelay::start().await,
            TestRelay::start().await,
            TestRelay::start().await,
        );
        let user = Keys::generate();
        let list = relay_list(
            &user,
            &[
                (&write.url, Some(RelayMetadata::Write)),
                (&read.url, Some(RelayMetadata::Read)),
            ],
        );
        index.relay.add_event(list).await.unwrap();
        let discovery = Discovery {
            index: vec![index.url.clone()],
            fallback: vec![read.url.clone()],
        };
        let account = Account::sign_in(Arc::new(user), &discovery).await.unwrap();
        assert_eq!(account.relays().urls(), std::slice::from_ref(&write.url));

        let stranger = Account::sign_in(Arc::new(Keys::generate()), &discovery)
            .await
            .unwrap();
        assert_eq!(stranger.relays().urls(), std::slice::from_ref(&read.url));
    }

    #[tokio::test]
    async fn opening_a_drive_reads_its_lists_and_keeps_the_descriptor() {
        let (user_relay, drive_relay) = (TestRelay::start().await, TestRelay::start().await);
        let user = Arc::new(Keys::generate());
        let servers = [blossom_server(None).await];
        let account = Account::connect(Arc::clone(&user), [user_relay.url.clone()])
            .await
            .unwrap();
        let (created, lists) = account
            .create_drive("Lupine", std::slice::from_ref(&drive_relay.url), &servers)
            .await
            .unwrap();

        let fresh = TestRelay::start().await;
        let device = Account::connect(user, [user_relay.url.clone(), fresh.url.clone()])
            .await
            .unwrap();
        let found = device.drives().await.unwrap().remove(0);
        assert_eq!(found.descriptor.name, "Lupine");
        assert_eq!(device.open(&found).await.unwrap(), lists);
        assert!(fresh.holds(created.event.id).await);

        let empty = TestRelay::start().await;
        let lost = Account::connect(Arc::new(Keys::generate()), [empty.url.clone()])
            .await
            .unwrap();
        assert!(lost.open(&found).await.is_err());
    }

    #[tokio::test]
    async fn drives_are_not_created_on_media_hosts() {
        let (user_relay, drive_relay) = (TestRelay::start().await, TestRelay::start().await);
        let account = Account::connect(Arc::new(Keys::generate()), [user_relay.url.clone()])
            .await
            .unwrap();
        let servers = [blossom_server(None).await, blossom_server(Some(415)).await];
        let refused = account
            .create_drive("Lupine", std::slice::from_ref(&drive_relay.url), &servers)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, Error::Http { status: 415, .. }),
            "{refused}"
        );
        assert!(
            account.drives().await.unwrap().is_empty(),
            "nothing announced"
        );
    }

    #[tokio::test]
    async fn descriptors_go_back_to_the_relays_that_lost_them() {
        let (user_relay, drive_relay) = (TestRelay::start().await, TestRelay::start().await);
        let user = Keys::generate();
        let descriptor = Descriptor::generate("Lupine");
        let event = descriptor.seal(&user).await.unwrap();
        let keys = descriptor.keys();
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: descriptor.id,
                pubkey: keys.public_key(),
                epoch: 0,
                relays: vec![drive_relay.url.clone()],
                servers: vec!["https://blossom.example.com".into()],
            })
            .unwrap();
        assert_eq!(keep_descriptor(&store, &keys).await.unwrap(), 0);

        store
            .set_descriptor(&DescriptorRecord {
                event: event.clone(),
                name: "Lupine".into(),
                user_relays: vec![user_relay.url.clone()],
            })
            .unwrap();
        assert_eq!(keep_descriptor(&store, &keys).await.unwrap(), 2);
        assert!(user_relay.holds(event.id).await && drive_relay.holds(event.id).await);
        assert_eq!(keep_descriptor(&store, &keys).await.unwrap(), 0);
    }

    struct Approve;

    impl NostrConnectSignerActions for Approve {
        fn approve(&self, _: &PublicKey, _: &NostrConnectRequest) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn a_remote_signer_creates_and_finds_drives() {
        let (signer_relay, user_relay, drive_relay) = (
            TestRelay::start().await,
            TestRelay::start().await,
            TestRelay::start().await,
        );
        let user = Keys::generate();
        let keys = NostrConnectKeys::new(Keys::generate(), user.clone());
        let bunker = NostrConnectRemoteSigner::new(
            keys,
            [signer_relay.url.clone()],
            Some("secret".into()),
            None,
        )
        .unwrap();
        let link = bunker.bunker_uri().to_string();
        tokio::spawn(async move { bunker.serve(Approve).await });
        // NIP-46 requests are ephemeral: one sent before the signer subscribes is lost.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let signer = Arc::new(UserKey::parse(&link, None).unwrap());
        assert_eq!(signer.public_key().await.unwrap(), user.public_key());
        let account = Account::connect(Arc::clone(&signer), [user_relay.url.clone()])
            .await
            .unwrap();
        let servers = [blossom_server(None).await];
        let (created, _) = account
            .create_drive("Remote", std::slice::from_ref(&drive_relay.url), &servers)
            .await
            .unwrap();
        assert_eq!(created.event.pubkey, user.public_key());

        let local = Account::connect(Arc::new(user), [user_relay.url.clone()])
            .await
            .unwrap();
        let found = local.drives().await.unwrap();
        assert_eq!(found[0].descriptor.secret, created.descriptor.secret);
        signer.shutdown().await;
    }
}
