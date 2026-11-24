//! Signing in: the user's key opens a drive or creates one, then this
//! device keeps only the drive's keys.

use std::sync::{Arc, Mutex};

use lupine_core::account::{Account, Discovery};
use lupine_core::descriptor::FoundDrive;
use lupine_core::identity::{NOSTR_CONNECT_RELAYS, OpenUrl, UserKey};
use lupine_core::store::{DescriptorRecord, DriveRecord};
use nostr::nips::nip19::ToBech32;
use nostr::types::RelayUrl;

use crate::CoreError;
use crate::local::{ImportedDrive, set_up};
use crate::reader::Cancellation;

#[uniffi::export(callback_interface)]
pub trait SignerApproval: Send + Sync {
    /// The remote signer asks the user to approve Lupine on this page.
    fn open_url(&self, url: String);
}

fn opener(approval: Box<dyn SignerApproval>) -> OpenUrl {
    let approval: Arc<dyn SignerApproval> = Arc::from(approval);
    Arc::new(move |url| approval.open_url(url.to_string()))
}

#[derive(Debug, uniffi::Record)]
pub struct RemoteDrive {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default, uniffi::Record)]
pub struct DriveLists {
    pub relays: Vec<String>,
    pub servers: Vec<String>,
}

/// The user, signed in until [`close`](Self::close).
#[derive(uniffi::Object)]
pub struct SignIn {
    account: Account<UserKey>,
    found: tokio::sync::Mutex<Vec<FoundDrive>>,
}

/// Signs in with an nsec or a `bunker://` link.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sign_in(
    key: String,
    approval: Box<dyn SignerApproval>,
) -> Result<Arc<SignIn>, CoreError> {
    SignIn::new(UserKey::parse(&key, Some(opener(approval)))?).await
}

/// A `nostrconnect://` link for a signer app to scan or open.
#[derive(uniffi::Object)]
pub struct SignerPairing {
    link: String,
    key: Mutex<Option<UserKey>>,
    cancel: Cancellation,
}

#[uniffi::export]
pub fn pair_signer(approval: Box<dyn SignerApproval>) -> Result<Arc<SignerPairing>, CoreError> {
    let relays = NOSTR_CONNECT_RELAYS
        .iter()
        .map(|url| RelayUrl::parse(url).expect("a valid relay URL"))
        .collect();
    let (link, key) = UserKey::pair(relays, Some(opener(approval)))?;
    Ok(Arc::new(SignerPairing {
        link,
        key: Mutex::new(Some(key)),
        cancel: Cancellation::default(),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl SignerPairing {
    pub fn link(&self) -> String {
        self.link.clone()
    }

    /// Waits for the user to accept in the signer, then signs in. Once
    /// only, until [`cancel`](Self::cancel).
    pub async fn accepted(&self) -> Result<Arc<SignIn>, CoreError> {
        let key = self
            .key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or(CoreError::Cancelled)?;
        let accepted = tokio::select! {
            () = self.cancel.cancelled() => Err(CoreError::Cancelled),
            accepted = key.public_key() => accepted.map_err(CoreError::from),
        };
        if let Err(e) = accepted {
            key.shutdown().await;
            return Err(e);
        }
        SignIn::new(key).await
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl SignIn {
    async fn new(key: UserKey) -> Result<Arc<Self>, CoreError> {
        let key = Arc::new(key);
        match Account::sign_in(Arc::clone(&key), &Discovery::default()).await {
            Ok(account) => Ok(Arc::new(Self {
                account,
                found: tokio::sync::Mutex::default(),
            })),
            Err(e) => {
                key.shutdown().await;
                Err(e.into())
            }
        }
    }

    fn set_up(
        &self,
        path: &str,
        found: &FoundDrive,
        lists: lupine_core::account::DriveLists,
    ) -> Result<ImportedDrive, CoreError> {
        let descriptor = &found.descriptor;
        let record = DriveRecord {
            id: descriptor.id,
            pubkey: descriptor.keys().public_key(),
            epoch: descriptor.epoch,
            relays: lists.relays,
            servers: lists.servers,
        };
        let kept = DescriptorRecord {
            event: found.event.clone(),
            name: descriptor.name.clone(),
            user_relays: self.account.relays().urls().to_vec(),
        };
        set_up(path, &record, Some(&kept))?;
        Ok(ImportedDrive {
            id: descriptor.id.to_hex(),
            secret: descriptor.secret.to_secret_hex(),
            recovery_key: descriptor.recovery_key(&record.relays).encode(),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SignIn {
    /// The user's npub.
    pub fn user(&self) -> String {
        npub(&self.account.public_key())
    }

    pub fn uses_remote_signer(&self) -> bool {
        self.account.signer().is_remote()
    }

    /// The user's drives, by name.
    pub async fn drives(&self) -> Result<Vec<RemoteDrive>, CoreError> {
        let found = self.account.drives().await?;
        let drives = found
            .iter()
            .map(|drive| RemoteDrive {
                id: drive.descriptor.id.to_hex(),
                name: drive.descriptor.name.clone(),
            })
            .collect();
        *self.found.lock().await = found;
        Ok(drives)
    }

    /// The user's own inbox relays and Blossom servers, to suggest for a
    /// new drive. Empty when the user lists none.
    pub async fn suggested_lists(&self) -> DriveLists {
        let lists = self.account.suggestions().await.unwrap_or_default();
        DriveLists {
            relays: lists.relays.iter().map(ToString::to_string).collect(),
            servers: lists.servers,
        }
    }

    /// Sets up the database at `path` for drive `id` of [`drives`](Self::drives).
    pub async fn open_drive(&self, path: String, id: String) -> Result<ImportedDrive, CoreError> {
        let found = self
            .found
            .lock()
            .await
            .iter()
            .find(|drive| drive.descriptor.id.to_hex() == id)
            .cloned()
            .ok_or(CoreError::NoSuchItem)?;
        let lists = self.account.open(&found).await?;
        self.set_up(&path, &found, lists)
    }

    /// Creates a drive and sets up the database at `path` for it. Relay and
    /// server addresses may leave out their scheme.
    pub async fn create_drive(
        &self,
        path: String,
        name: String,
        relays: Vec<String>,
        servers: Vec<String>,
    ) -> Result<ImportedDrive, CoreError> {
        let relays = relays
            .iter()
            .filter_map(|url| with_scheme(url, "wss://"))
            .map(|url| {
                RelayUrl::parse(&url).map_err(|_| CoreError::Check(format!("invalid relay: {url}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let servers: Vec<String> = servers
            .iter()
            .filter_map(|url| with_scheme(url, "https://"))
            .collect();
        let name = match name.trim() {
            "" => "Lupine",
            name => name,
        };
        let (found, lists) = self.account.create_drive(name, &relays, &servers).await?;
        self.set_up(&path, &found, lists)
    }

    pub async fn close(&self) {
        self.account.shutdown().await;
        self.account.signer().shutdown().await;
    }
}

fn with_scheme(url: &str, scheme: &str) -> Option<String> {
    match url.trim() {
        "" => None,
        url if url.contains("://") => Some(url.to_owned()),
        url => Some(format!("{scheme}{url}")),
    }
}

pub(crate) fn npub(key: &nostr::key::PublicKey) -> String {
    key.to_bech32().expect("a public key encodes")
}
