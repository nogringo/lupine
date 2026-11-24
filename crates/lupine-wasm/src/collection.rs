//! Deleting from the drive's servers the blobs no version references any
//! more.

use lupine_core::collection;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::local::LocalDrive;
use crate::reader::Cancellation;
use crate::{CoreError, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface CollectionReport {
    /** Deleted from every drive server. */
    deleted: number;
    /** Nothing was deleted: the drive holds a version this device cannot read, which may reference any blob. */
    heldBack: boolean;
    /** Left for the next collection, with the reason. */
    failed: string[];
}
"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionReport {
    deleted: usize,
    held_back: bool,
    failed: Vec<String>,
}

/// Deletes from the drive's servers the blobs that no version has
/// referenced for 30 days. Run right after the drive session synced every
/// relay, until `cancellation`.
#[wasm_bindgen(js_name = collectBlobs, unchecked_return_type = "CollectionReport")]
pub async fn collect_blobs(
    drive: &LocalDrive,
    secret: String,
    cancellation: &Cancellation,
) -> Result<JsValue, CoreError> {
    let keys = drive.keys(&secret)?;
    let collected = tokio::select! {
        () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        collected = collection::collect(&drive.store, &keys) => collected?,
    };
    to_js(&CollectionReport {
        deleted: collected.deleted,
        held_back: collected.held_back,
        failed: collected
            .failed
            .iter()
            .map(|(hash, e)| format!("{hash}: {e}"))
            .collect(),
    })
}
