//! The local database of a drive, and setting it up from a recovery key.

use std::sync::Arc;
use std::time::Duration;

use lupine_core::account::DriveLists;
use lupine_core::blossom::BlossomStore;
use lupine_core::descriptor::RecoveryKey;
use lupine_core::drive::DriveKeys;
use lupine_core::ids::NodeId;
use lupine_core::items::Item;
use lupine_core::relays::Relays;
use lupine_core::store::{DescriptorRecord, DriveRecord, Store};
use nostr::key::SecretKey;
use nostr::nips::nip19::ToBech32;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::storage::remove_database;
use crate::{CoreError, to_js};

const DAY_SECS: u64 = 24 * 60 * 60;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface DriveInfo {
    id: string;
    /** Absent for a drive opened with a recovery key. */
    name?: string;
    /** The drive key, as an npub: paid relays and servers must allow it. */
    driveKey: string;
    /** The npub this device signed in with, absent with a recovery key. */
    user?: string;
    relays: string[];
    servers: string[];
    /** Servers the drive left, still read from until every blob reached the others. */
    leaving: string[];
    /** Blobs are still to be copied to the servers. */
    filling: boolean;
    epoch: number;
    /** The key a rotation retired, as an npub, until every wrap addressed to it is deleted. */
    retiringDriveKey?: string;
}

export interface DriveItem {
    id: string;
    /** Absent at the drive root. */
    parent?: string;
    name: string;
    isFolder: boolean;
    size: number;
    /** Unix milliseconds. */
    created?: number;
    modified?: number;
    /** Hex, absent for folders. */
    contentVersion?: string;
}

export interface ImportedDrive {
    id: string;
    /** The drive secret, hex, to keep. */
    secret: string;
    recoveryKey: string;
}
"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriveInfo {
    id: String,
    name: Option<String>,
    drive_key: String,
    user: Option<String>,
    relays: Vec<String>,
    servers: Vec<String>,
    leaving: Vec<String>,
    filling: bool,
    epoch: u32,
    retiring_drive_key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DriveItem {
    id: String,
    parent: Option<String>,
    name: String,
    is_folder: bool,
    size: u64,
    created: Option<i64>,
    modified: Option<i64>,
    content_version: Option<String>,
}

impl From<Item> for DriveItem {
    fn from(item: Item) -> Self {
        Self {
            id: item.node.to_hex(),
            parent: item.parent.map(|parent| parent.to_hex()),
            is_folder: item.is_folder(),
            size: item.size(),
            created: item.created,
            modified: item.modified,
            content_version: item.content_version().map(hex::encode),
            name: item.name,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedDrive {
    pub(crate) id: String,
    pub(crate) secret: String,
    pub(crate) recovery_key: String,
}

#[wasm_bindgen]
pub struct LocalDrive {
    pub(crate) store: Arc<Store>,
}

/// The drive set up in the database at `path`, `undefined` when there is
/// none.
#[wasm_bindgen(js_name = openLocalDrive)]
pub fn open_local_drive(path: String) -> Result<Option<LocalDrive>, CoreError> {
    let store = Store::open(&path)?;
    let set_up = store.drive()?.is_some();
    Ok(set_up.then(|| LocalDrive {
        store: Arc::new(store),
    }))
}

/// Deletes the database at `path`. Free every `LocalDrive` on it first.
#[wasm_bindgen(js_name = removeLocalDrive)]
pub fn remove_local_drive(path: String) -> Result<(), CoreError> {
    remove_database(&path)
}

/// The drive key a drive secret, in hex, opens, as an npub.
#[wasm_bindgen(js_name = driveKey)]
pub fn drive_key(secret: String) -> Result<String, CoreError> {
    Ok(npub(&secret_keys(&secret)?.public_key()))
}

/// Sets up the database at `path` for the drive a recovery key opens. The
/// drive's lists come from the relays the key names.
#[wasm_bindgen(js_name = importRecoveryKey, unchecked_return_type = "ImportedDrive")]
pub async fn import_recovery_key(path: String, recovery_key: String) -> Result<JsValue, CoreError> {
    let key = RecoveryKey::decode(&recovery_key)?;
    if key.relays.is_empty() {
        return Err(CoreError::Check("the recovery key names no relay".into()));
    }
    let keys = key.keys();
    let relays = Relays::for_drive(&keys, key.relays.clone()).await?;
    let found = DriveLists::find(&relays, keys.public_key()).await;
    relays.shutdown().await;
    let lists = found?;
    if lists.servers.is_empty() {
        return Err(CoreError::Check("the drive lists no Blossom server".into()));
    }
    let record = DriveRecord {
        id: key.id,
        pubkey: keys.public_key(),
        epoch: key.epoch,
        relays: if lists.relays.is_empty() {
            key.relays.clone()
        } else {
            lists.relays
        },
        servers: lists.servers,
    };
    set_up(&path, &record, None)?;
    to_js(&ImportedDrive {
        id: key.id.to_hex(),
        secret: key.secret.to_secret_hex(),
        recovery_key: key.encode(),
    })
}

/// Sets up the database at `path` for `drive`. One that holds another drive,
/// or another key of it, is replaced: moving a database to a rotated key,
/// as the Apple app does, needs the scratch database the web lacks yet.
pub(crate) fn set_up(
    path: &str,
    drive: &DriveRecord,
    descriptor: Option<&DescriptorRecord>,
) -> Result<(), CoreError> {
    let mut store = Store::open(path)?;
    let held = store.drive()?;
    if let Some(held) = &held {
        if held.id == drive.id && held.epoch > drive.epoch {
            return Err(CoreError::Check(
                "this key is from before the drive's last key rotation".into(),
            ));
        }
        if held.id != drive.id || held.pubkey != drive.pubkey {
            drop(store);
            remove_database(path)?;
            store = Store::open(path)?;
        }
    }
    store.set_drive(drive)?;
    if let Some(descriptor) = descriptor {
        store.set_descriptor(descriptor)?;
    }
    Ok(())
}

pub(crate) fn secret_keys(secret: &str) -> Result<DriveKeys, CoreError> {
    let secret =
        SecretKey::from_hex(secret).map_err(|_| CoreError::Check("invalid drive secret".into()))?;
    Ok(DriveKeys::from_secret(secret))
}

#[wasm_bindgen]
impl LocalDrive {
    #[wasm_bindgen(unchecked_return_type = "DriveInfo")]
    pub fn info(&self) -> Result<JsValue, CoreError> {
        let drive = self.record()?;
        let descriptor = self.store.descriptor()?;
        let lists = self.store.lists_state()?;
        let retiring = self.store.retiring()?;
        to_js(&DriveInfo {
            id: drive.id.to_hex(),
            name: descriptor.as_ref().map(|d| d.name.clone()),
            drive_key: npub(&drive.pubkey),
            user: descriptor.as_ref().map(|d| npub(&d.event.pubkey)),
            relays: drive.relays.iter().map(ToString::to_string).collect(),
            servers: drive.servers,
            leaving: lists.leaving,
            filling: lists.fill != 0,
            epoch: drive.epoch,
            retiring_drive_key: retiring.map(|(pubkey, _)| npub(&pubkey)),
        })
    }

    /// Full access to the drive without the user's key, for safekeeping.
    #[wasm_bindgen(js_name = recoveryKey)]
    pub fn recovery_key(&self, secret: String) -> Result<String, CoreError> {
        let keys = self.keys(&secret)?;
        let drive = self.record()?;
        let key = RecoveryKey {
            secret: keys.keys().secret_key().clone(),
            id: drive.id,
            epoch: drive.epoch,
            relays: drive.relays,
        };
        Ok(key.encode())
    }

    /// How many days the relays keep versions that a newer one replaced,
    /// before this device compacts them.
    #[wasm_bindgen(js_name = historyDays)]
    pub fn history_days(&self) -> Result<u32, CoreError> {
        Ok((self.store.history()?.as_secs() / DAY_SECS) as u32)
    }

    #[wasm_bindgen(js_name = setHistoryDays)]
    pub fn set_history_days(&self, days: u32) -> Result<(), CoreError> {
        if days == 0 {
            return Err(CoreError::Check(
                "the history window is at least a day".into(),
            ));
        }
        let window = Duration::from_secs(u64::from(days) * DAY_SECS);
        Ok(self.store.set_history(window)?)
    }

    /// Changes that no relay took yet.
    pub fn unsent(&self) -> Result<u32, CoreError> {
        Ok(self.store.unsent()? as u32)
    }

    #[wasm_bindgen(unchecked_return_type = "DriveItem")]
    pub fn item(&self, id: String) -> Result<JsValue, CoreError> {
        let item = self.store.item(&node(&id)?)?;
        to_js(&item.map(DriveItem::from).ok_or(CoreError::NoSuchItem)?)
    }

    /// The items in `parent` (`undefined` for the drive root) after `after`.
    #[wasm_bindgen(unchecked_return_type = "DriveItem[]")]
    pub fn children(
        &self,
        parent: Option<String>,
        after: Option<String>,
        limit: u32,
    ) -> Result<JsValue, CoreError> {
        let parent = parent.as_deref().map(node).transpose()?;
        let after = after.as_deref().map(node).transpose()?;
        let items = self.store.children(parent, after, limit as usize)?;
        to_js(&items.into_iter().map(DriveItem::from).collect::<Vec<_>>())
    }
}

impl LocalDrive {
    pub(crate) fn record(&self) -> Result<DriveRecord, CoreError> {
        self.store
            .drive()?
            .ok_or_else(|| CoreError::Check("no drive is set up".into()))
    }

    /// The drive's servers, and those it leaves as sources.
    pub(crate) fn blobs(&self, keys: &DriveKeys) -> Result<BlossomStore, CoreError> {
        let leaving = self.store.lists_state()?.leaving;
        Ok(BlossomStore::new(&self.record()?.servers, keys.keys())?
            .with_sources(&leaving, keys.keys())?)
    }

    /// The keys of this drive, from its secret in hex.
    pub(crate) fn keys(&self, secret: &str) -> Result<DriveKeys, CoreError> {
        let keys = secret_keys(secret)?;
        if keys.public_key() != self.record()?.pubkey {
            return Err(CoreError::Check(
                "this secret does not open the drive".into(),
            ));
        }
        Ok(keys)
    }
}

/// An id that is not a node id names no item.
pub(crate) fn node(id: &str) -> Result<NodeId, CoreError> {
    id.parse().map_err(|_| CoreError::NoSuchItem)
}

pub(crate) fn npub(key: &nostr::key::PublicKey) -> String {
    key.to_bech32().expect("a public key encodes")
}
