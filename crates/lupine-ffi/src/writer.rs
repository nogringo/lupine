//! Finder's changes, from the File Provider extension: creations, edits,
//! moves and deletions, written to the local database for the app to send.

use std::path::Path;
use std::sync::Arc;

use lupine_core::blossom::BlossomStore;
use lupine_core::ids::NodeId;
use lupine_core::writer::{self as core_writer, Contents, Edit, Place};

use crate::CoreError;
use crate::local::{DriveItem, LocalDrive, node};
use crate::reader::{Cancellation, TransferObserver};

#[derive(Debug, uniffi::Record)]
pub struct NewItem {
    /// The system's identifier for this creation, kept when it replays it.
    pub request: String,
    /// `None` at the drive root.
    pub parent: Option<String>,
    pub name: String,
    pub is_folder: bool,
    /// An import: the item may match one of the same name.
    pub may_already_exist: bool,
    /// The path of a file's content.
    pub contents: Option<String>,
    /// Unix milliseconds.
    pub created: Option<i64>,
    pub modified: Option<i64>,
}

#[derive(Debug, uniffi::Record)]
pub struct ItemLocation {
    /// `None` at the drive root.
    pub parent: Option<String>,
    pub name: String,
}

#[derive(Debug, uniffi::Record)]
pub struct ItemEdit {
    pub id: String,
    pub is_folder: bool,
    /// The content version the edit was made on. Empty for folders.
    pub base_content_version: Vec<u8>,
    /// Set for a move or a rename.
    pub location: Option<ItemLocation>,
    /// The path of a file's new content.
    pub contents: Option<String>,
    /// Unix milliseconds. `None` keeps the current value.
    pub created: Option<i64>,
    pub modified: Option<i64>,
}

#[derive(uniffi::Object)]
pub struct ItemWriter {
    writer: core_writer::ItemWriter<BlossomStore>,
}

#[uniffi::export]
pub fn open_item_writer(
    drive: Arc<LocalDrive>,
    secret: String,
) -> Result<Arc<ItemWriter>, CoreError> {
    let keys = drive.keys(&secret)?;
    let blobs = drive.blobs(&keys)?;
    let writer = core_writer::ItemWriter::new(Arc::clone(&drive.store), keys, blobs)?;
    Ok(Arc::new(ItemWriter { writer }))
}

#[uniffi::export(async_runtime = "tokio")]
impl ItemWriter {
    /// `None` when an import matches no item and brings no content.
    pub async fn create_item(
        &self,
        item: NewItem,
        observer: Box<dyn TransferObserver>,
        cancellation: Arc<Cancellation>,
    ) -> Result<Option<DriveItem>, CoreError> {
        let new = core_writer::NewItem {
            request: item.request,
            place: Place {
                parent: parent(item.parent.as_deref())?,
                name: item.name,
            },
            folder: item.is_folder,
            may_exist: item.may_already_exist,
            created: item.created,
            modified: item.modified,
        };
        let progress = |bytes| observer.progressed(bytes);
        let contents = item
            .contents
            .as_deref()
            .map(|path| Contents::File(Path::new(path)));
        let created = self.writer.create(new, contents, &progress);
        let created = tokio::select! {
            created = created => created?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        Ok(created.map(DriveItem::from))
    }

    pub async fn modify_item(
        &self,
        edit: ItemEdit,
        observer: Box<dyn TransferObserver>,
        cancellation: Arc<Cancellation>,
    ) -> Result<DriveItem, CoreError> {
        let place = edit
            .location
            .map(|location| {
                Ok::<_, CoreError>(Place {
                    parent: parent(location.parent.as_deref())?,
                    name: location.name,
                })
            })
            .transpose()?;
        let change = Edit {
            base_content: content_version(&edit.base_content_version),
            place,
            created: edit.created,
            modified: edit.modified,
        };
        let progress = |bytes| observer.progressed(bytes);
        let contents = edit
            .contents
            .as_deref()
            .map(|path| Contents::File(Path::new(path)));
        let modified =
            self.writer
                .modify(node(&edit.id)?, edit.is_folder, change, contents, &progress);
        let modified = tokio::select! {
            modified = modified => modified?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        Ok(modified.into())
    }

    /// Deletes the item and, when `recursive`, everything under it. A file
    /// that changed since `base_content_version` is kept, and enumerated
    /// again.
    pub fn delete_item(
        &self,
        id: String,
        base_content_version: Vec<u8>,
        recursive: bool,
    ) -> Result<(), CoreError> {
        let Ok(node) = id.parse::<NodeId>() else {
            return Ok(());
        };
        self.writer
            .delete(node, content_version(&base_content_version), recursive)?;
        Ok(())
    }
}

/// An unknown parent id may only be late.
fn parent(id: Option<&str>) -> Result<Option<NodeId>, CoreError> {
    id.map(|id| id.parse().map_err(|_| CoreError::NoSuchParent))
        .transpose()
}

/// Anything but a content version matches no content.
fn content_version(bytes: &[u8]) -> Option<[u8; 32]> {
    bytes.try_into().ok()
}
