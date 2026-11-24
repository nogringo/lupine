//! The local database of a drive, which the app and the File Provider
//! extension share, and setting it up from a recovery key.

use std::sync::Arc;

use lupine_core::account::DriveLists;
use lupine_core::descriptor::RecoveryKey;
use lupine_core::drive::DriveKeys;
use lupine_core::ids::NodeId;
use lupine_core::items::Item;
use lupine_core::relays::Relays;
use lupine_core::store::{DescriptorRecord, DriveRecord, Store};
use nostr::key::SecretKey;

use crate::CoreError;
use crate::account::npub;

#[derive(uniffi::Object)]
pub struct LocalDrive {
    pub(crate) store: Arc<Store>,
}

#[derive(Debug, uniffi::Record)]
pub struct DriveInfo {
    pub id: String,
    /// `None` for a drive opened with a recovery key.
    pub name: Option<String>,
    /// The drive key, as an npub: paid relays and servers must allow it.
    pub drive_key: String,
    /// The npub this device signed in with, `None` with a recovery key.
    pub user: Option<String>,
    pub relays: Vec<String>,
    pub servers: Vec<String>,
}

#[derive(Debug, uniffi::Record)]
pub struct DriveItem {
    pub id: String,
    /// `None` at the drive root.
    pub parent: Option<String>,
    pub name: String,
    pub is_folder: bool,
    pub size: u64,
    /// Unix milliseconds.
    pub created: Option<i64>,
    pub modified: Option<i64>,
    /// Empty for folders.
    pub content_version: Vec<u8>,
    pub metadata_version: Vec<u8>,
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
            content_version: item.content_version().map(Vec::from).unwrap_or_default(),
            metadata_version: item.metadata_version().to_vec(),
            name: item.name,
        }
    }
}

#[derive(Debug, uniffi::Record)]
pub struct ItemChanges {
    pub updated: Vec<DriveItem>,
    pub removed: Vec<String>,
    pub anchor: Vec<u8>,
    pub more: bool,
}

#[derive(Debug, uniffi::Record)]
pub struct ImportedDrive {
    pub id: String,
    /// The drive secret, hex, for the keychain.
    pub secret: String,
    pub recovery_key: String,
}

/// The drive set up in the database at `path`, `None` when there is none.
#[uniffi::export]
pub fn open_local_drive(path: String) -> Result<Option<Arc<LocalDrive>>, CoreError> {
    let store = Store::open(path)?;
    let set_up = store.drive()?.is_some();
    Ok(set_up.then(|| {
        Arc::new(LocalDrive {
            store: Arc::new(store),
        })
    }))
}

/// Sets up the database at `path` for the drive a recovery key opens. The
/// drive's lists come from the relays the key names.
#[uniffi::export(async_runtime = "tokio")]
pub async fn import_recovery_key(
    path: String,
    recovery_key: String,
) -> Result<ImportedDrive, CoreError> {
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
    Ok(ImportedDrive {
        id: key.id.to_hex(),
        secret: key.secret.to_secret_hex(),
        recovery_key: key.encode(),
    })
}

/// Sets up the database at `path` for `drive`. A database that holds
/// another drive, whose key this device no longer has, is replaced.
pub(crate) fn set_up(
    path: &str,
    drive: &DriveRecord,
    descriptor: Option<&DescriptorRecord>,
) -> Result<(), CoreError> {
    let mut store = Store::open(path)?;
    if store.drive()?.is_some_and(|held| held.id != drive.id) {
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            match std::fs::remove_file(format!("{path}{suffix}")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(CoreError::Core(e.to_string()));
                }
                _ => {}
            }
        }
        store = Store::open(path)?;
    }
    store.set_drive(drive)?;
    if let Some(descriptor) = descriptor {
        store.set_descriptor(descriptor)?;
    }
    Ok(())
}

#[uniffi::export]
impl LocalDrive {
    pub fn info(&self) -> Result<DriveInfo, CoreError> {
        let drive = self.record()?;
        let descriptor = self.store.descriptor()?;
        Ok(DriveInfo {
            id: drive.id.to_hex(),
            name: descriptor.as_ref().map(|d| d.name.clone()),
            drive_key: npub(&drive.pubkey),
            user: descriptor.as_ref().map(|d| npub(&d.event.pubkey)),
            relays: drive.relays.iter().map(ToString::to_string).collect(),
            servers: drive.servers,
        })
    }

    /// Full access to the drive without the user's key, for safekeeping.
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

    /// Changes from Finder and folder sync that no relay took yet.
    pub fn unsent(&self) -> Result<u64, CoreError> {
        Ok(self.store.unsent()?)
    }

    pub fn item(&self, id: String) -> Result<DriveItem, CoreError> {
        let item = self.store.item(&node(&id)?)?;
        item.map(DriveItem::from).ok_or(CoreError::NoSuchItem)
    }

    /// The items in `parent` (`None` for the drive root) after `after`.
    pub fn children(
        &self,
        parent: Option<String>,
        after: Option<String>,
        limit: u32,
    ) -> Result<Vec<DriveItem>, CoreError> {
        let parent = parent.as_deref().map(node).transpose()?;
        let after = after.as_deref().map(node).transpose()?;
        let items = self.store.children(parent, after, limit as usize)?;
        Ok(items.into_iter().map(DriveItem::from).collect())
    }

    /// Every item after `after`.
    pub fn all_items(
        &self,
        after: Option<String>,
        limit: u32,
    ) -> Result<Vec<DriveItem>, CoreError> {
        let after = after.as_deref().map(node).transpose()?;
        let items = self.store.items(after, limit as usize)?;
        Ok(items.into_iter().map(DriveItem::from).collect())
    }

    pub fn anchor(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.store.anchor()?.to_bytes().to_vec())
    }

    pub fn changes(&self, anchor: Vec<u8>, limit: u32) -> Result<ItemChanges, CoreError> {
        let changes = self.store.changes(&anchor, limit as usize)?;
        Ok(ItemChanges {
            updated: changes.updated.into_iter().map(DriveItem::from).collect(),
            removed: changes.removed.iter().map(NodeId::to_hex).collect(),
            anchor: changes.anchor.to_bytes().to_vec(),
            more: changes.more,
        })
    }
}

impl LocalDrive {
    pub(crate) fn record(&self) -> Result<DriveRecord, CoreError> {
        self.store
            .drive()?
            .ok_or_else(|| CoreError::Check("no drive is set up".into()))
    }

    /// The keys of this drive, from its secret in hex.
    pub(crate) fn keys(&self, secret: &str) -> Result<DriveKeys, CoreError> {
        let secret = SecretKey::from_hex(secret)
            .map_err(|_| CoreError::Check("invalid drive secret".into()))?;
        let keys = DriveKeys::from_secret(secret);
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
