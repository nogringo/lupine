//! The user's side of a drive, on the user's write relays (NIP-65): the
//! descriptors that hold drive secrets, and the drives' lists of relays and
//! Blossom servers, which a new device needs before it can open a drive.

use std::sync::Arc;

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::types::RelayUrl;

use crate::descriptor::{self, DESCRIPTOR_KIND, Descriptor, FoundDrive, UserSigner};
use crate::drive::{read_relay_list, read_server_list};
use crate::error::{Error, Result};
use crate::relays::{Relays, Sent};

/// Where a drive keeps its versions and its blobs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriveLists {
    pub relays: Vec<RelayUrl>,
    /// Most trusted first.
    pub servers: Vec<String>,
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

    /// Creates a drive and announces it: the descriptor to the user's
    /// relays and the drive relays, and the drive's lists to both.
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
        let event = descriptor.seal(&*self.signer).await?;
        let keys = descriptor.keys();
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
        let filter = Filter::new()
            .kinds([Kind::InboxRelays, Kind::BlossomServerList])
            .author(drive);
        let events = self.relays.query(&filter).await?;
        let latest = |kind: Kind| {
            events
                .iter()
                .filter(|e| e.kind == kind && e.verify().is_ok())
                .max_by_key(|e| (e.created_at, std::cmp::Reverse(e.id)))
        };
        Ok(DriveLists {
            relays: latest(Kind::InboxRelays)
                .map(read_relay_list)
                .unwrap_or_default(),
            servers: latest(Kind::BlossomServerList)
                .map(read_server_list)
                .unwrap_or_default(),
        })
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
