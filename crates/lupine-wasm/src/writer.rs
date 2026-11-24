//! The page's changes: creations, edits, moves and deletions, written to the
//! local database for the drive session to send.

use std::sync::Arc;

use js_sys::Function;
use lupine_core::blossom::BlossomStore;
use lupine_core::ids::NodeId;
use lupine_core::writer::{self as core_writer, Contents, Edit, Place};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use web_sys::Blob;

use crate::io::BlobSource;
use crate::local::{DriveItem, LocalDrive, node};
use crate::reader::{Cancellation, progress};
use crate::{CoreError, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface NewItem {
    /** The page's id for this creation: replayed, it finds the item it made. */
    request: string;
    /** Absent at the drive root. */
    parent?: string;
    name: string;
    isFolder: boolean;
    /** Unix milliseconds. */
    created?: number;
    modified?: number;
}

export interface ItemEdit {
    id: string;
    isFolder: boolean;
    /** The content version the edit was made on, absent for folders. */
    baseContentVersion?: string;
    /** Set for a move or a rename. */
    location?: { parent?: string; name: string };
    /** Unix milliseconds. Absent keeps the current value. */
    created?: number;
    modified?: number;
}
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewItem {
    request: String,
    parent: Option<String>,
    name: String,
    is_folder: bool,
    created: Option<i64>,
    modified: Option<i64>,
}

#[derive(Deserialize)]
struct ItemLocation {
    parent: Option<String>,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemEdit {
    id: String,
    is_folder: bool,
    base_content_version: Option<String>,
    location: Option<ItemLocation>,
    created: Option<i64>,
    modified: Option<i64>,
}

#[wasm_bindgen]
pub struct ItemWriter {
    writer: core_writer::ItemWriter<BlossomStore>,
}

#[wasm_bindgen(js_name = openItemWriter)]
pub fn open_item_writer(drive: &LocalDrive, secret: String) -> Result<ItemWriter, CoreError> {
    let keys = drive.keys(&secret)?;
    let blobs = drive.blobs(&keys)?;
    let writer = core_writer::ItemWriter::new(Arc::clone(&drive.store), keys, blobs)?;
    Ok(ItemWriter { writer })
}

#[wasm_bindgen]
impl ItemWriter {
    /// A folder, or a file with `contents`.
    #[wasm_bindgen(js_name = createItem, unchecked_return_type = "DriveItem")]
    pub async fn create_item(
        &self,
        #[wasm_bindgen(unchecked_param_type = "NewItem")] item: JsValue,
        contents: Option<Blob>,
        #[wasm_bindgen(unchecked_param_type = "(bytes: number) => void")] observer: Function,
        cancellation: &Cancellation,
    ) -> Result<JsValue, CoreError> {
        let item: NewItem = serde_wasm_bindgen::from_value(item)?;
        let new = core_writer::NewItem {
            request: item.request,
            place: Place {
                parent: parent(item.parent.as_deref())?,
                name: item.name,
            },
            folder: item.is_folder,
            may_exist: false,
            created: item.created,
            modified: item.modified,
        };
        let contents = contents.map(source).transpose()?;
        let progress = progress(&observer);
        let created = self.writer.create(new, contents, &progress);
        let created = tokio::select! {
            created = created => created?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        to_js(&created.map(DriveItem::from).ok_or(CoreError::NoSuchItem)?)
    }

    /// Moves, renames or touches an item, and replaces a file's content
    /// with `contents`.
    #[wasm_bindgen(js_name = modifyItem, unchecked_return_type = "DriveItem")]
    pub async fn modify_item(
        &self,
        #[wasm_bindgen(unchecked_param_type = "ItemEdit")] edit: JsValue,
        contents: Option<Blob>,
        #[wasm_bindgen(unchecked_param_type = "(bytes: number) => void")] observer: Function,
        cancellation: &Cancellation,
    ) -> Result<JsValue, CoreError> {
        let edit: ItemEdit = serde_wasm_bindgen::from_value(edit)?;
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
            base_content: edit
                .base_content_version
                .as_deref()
                .and_then(content_version),
            place,
            created: edit.created,
            modified: edit.modified,
        };
        let contents = contents.map(source).transpose()?;
        let progress = progress(&observer);
        let modified =
            self.writer
                .modify(node(&edit.id)?, edit.is_folder, change, contents, &progress);
        let modified = tokio::select! {
            modified = modified => modified?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        to_js(&DriveItem::from(modified))
    }

    /// Deletes the item and, when `recursive`, everything under it. A file
    /// that changed since `base_content_version` is kept.
    #[wasm_bindgen(js_name = deleteItem)]
    pub fn delete_item(
        &self,
        id: String,
        base_content_version: Option<String>,
        recursive: bool,
    ) -> Result<(), CoreError> {
        let Ok(node) = id.parse::<NodeId>() else {
            return Ok(());
        };
        let base = base_content_version.as_deref().and_then(content_version);
        self.writer.delete(node, base, recursive)?;
        Ok(())
    }
}

fn source(blob: Blob) -> Result<Contents<'static>, CoreError> {
    let source = BlobSource::new(blob).map_err(|e| CoreError::Core(e.to_string()))?;
    Ok(Contents::Reader(Arc::new(source)))
}

/// An unknown parent id may only be late.
fn parent(id: Option<&str>) -> Result<Option<NodeId>, CoreError> {
    id.map(|id| id.parse().map_err(|_| CoreError::NoSuchParent))
        .transpose()
}

/// Anything but a content version matches no content.
fn content_version(text: &str) -> Option<[u8; 32]> {
    hex::decode(text).ok()?.try_into().ok()
}
