//! Editing the drive's relays and servers, and copying every blob to the
//! servers the drive moves to.

use js_sys::Function;
use lupine_core::lists;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::account::{relay_urls, server_urls};
use crate::local::LocalDrive;
use crate::reader::Cancellation;
use crate::{CoreError, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface ListsChange {
    relays: boolean;
    servers: boolean;
}

export interface FillReport {
    blobs: number;
    /** On no server at all. */
    lost: number;
    /** Left for the next fill, with the reason. */
    failed: string[];
}
"#;

#[derive(Serialize)]
struct ListsChange {
    relays: bool,
    servers: bool,
}

#[derive(Serialize)]
struct FillReport {
    blobs: usize,
    lost: usize,
    failed: Vec<String>,
}

/// Publishes the drive's new relays and servers, one address per entry,
/// then applies them to the local database: open the drive again. Servers
/// added must take encrypted chunks.
#[wasm_bindgen(js_name = changeLists, unchecked_return_type = "ListsChange")]
pub async fn change_lists(
    drive: &LocalDrive,
    secret: String,
    relays: Vec<String>,
    servers: Vec<String>,
) -> Result<JsValue, CoreError> {
    let keys = drive.keys(&secret)?;
    let (relays, servers) = (relay_urls(&relays)?, server_urls(&servers));
    let changed = lists::change(&drive.store, &keys, relays, servers).await?;
    to_js(&ListsChange {
        relays: changed.relays,
        servers: changed.servers,
    })
}

/// Copies every blob the drive references to each of its servers that
/// lacks it. Run after the drive session synced every relay, while
/// `DriveInfo.filling` holds, until `cancellation`.
#[wasm_bindgen(js_name = fillServers, unchecked_return_type = "FillReport")]
pub async fn fill_servers(
    drive: &LocalDrive,
    secret: String,
    #[wasm_bindgen(unchecked_param_type = "(done: number, total: number) => void")]
    observer: Function,
    cancellation: &Cancellation,
) -> Result<JsValue, CoreError> {
    let keys = drive.keys(&secret)?;
    let progress = |done: usize, total: usize| {
        let _ = observer.call2(&JsValue::NULL, &done.into(), &total.into());
    };
    let filled = tokio::select! {
        () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        filled = lists::fill(&drive.store, &keys, progress) => filled?,
    };
    to_js(&FillReport {
        blobs: filled.blobs,
        lost: filled.lost.len(),
        failed: filled
            .failed
            .iter()
            .map(|(hash, e)| format!("{hash}: {e}"))
            .collect(),
    })
}
