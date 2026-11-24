//! Keeping the local database up to date with the relays, and sending what
//! the page wrote.

use std::sync::Arc;

use js_sys::Function;
use lupine_core::account;
use lupine_core::drive::DriveKeys;
use lupine_core::session::{DriveSession, Followed};
use lupine_core::store::Store;
use serde::Serialize;
use tokio::sync::{Mutex, Notify};
use wasm_bindgen::prelude::*;

use crate::local::LocalDrive;
use crate::reader::Cancellation;
use crate::{CoreError, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/**
 * `synced.failed` holds each relay that could not be synced, with the reason.
 * `listsChanged`: another device changed the drive's relays or servers, open
 * the drive again. `retired`: a key rotation made elsewhere retired the drive
 * key, following stopped and the drive needs the user's key, or a new
 * recovery key, to open again.
 */
export type SyncEvent =
    | { type: "syncing" }
    | { type: "synced"; failed: string[] }
    | { type: "compacted"; wraps: number }
    | { type: "listsChanged" }
    | { type: "itemsChanged" }
    | { type: "retired" };
"#;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SyncEvent {
    Syncing,
    Synced { failed: Vec<String> },
    Compacted { wraps: usize },
    ListsChanged,
    ItemsChanged,
    Retired,
}

#[wasm_bindgen]
pub struct DriveSync {
    session: Mutex<DriveSession>,
    store: Arc<Store>,
    keys: DriveKeys,
    stop: Cancellation,
    written: Notify,
}

#[wasm_bindgen(js_name = openDriveSync)]
pub async fn open_drive_sync(drive: &LocalDrive, secret: String) -> Result<DriveSync, CoreError> {
    let session = DriveSession::open(Arc::clone(&drive.store), drive.keys(&secret)?).await?;
    Ok(DriveSync {
        session: Mutex::new(session),
        store: Arc::clone(&drive.store),
        keys: drive.keys(&secret)?,
        stop: Cancellation::default(),
        written: Notify::new(),
    })
}

#[wasm_bindgen]
impl DriveSync {
    /// Keeps the local database up to date until [`stop`](Self::stop).
    pub async fn run(
        &self,
        #[wasm_bindgen(unchecked_param_type = "(event: SyncEvent) => void")] observer: Function,
    ) -> Result<(), CoreError> {
        let report = |event: SyncEvent| {
            if let Ok(event) = to_js(&event) {
                let _ = observer.call1(&JsValue::NULL, &event);
            }
        };
        let mut session = self.session.lock().await;
        let followed = session
            .follow(self.stop.cancelled(), &self.written, |event| match event {
                Followed::Syncing => report(SyncEvent::Syncing),
                Followed::Synced(sync) => report(SyncEvent::Synced {
                    failed: sync
                        .failed
                        .iter()
                        .map(|(relay, e)| format!("{relay}: {e}"))
                        .collect(),
                }),
                Followed::Compacted(wraps) => report(SyncEvent::Compacted { wraps }),
                Followed::ListsChanged => report(SyncEvent::ListsChanged),
                Followed::ItemsChanged => report(SyncEvent::ItemsChanged),
                Followed::Retired => report(SyncEvent::Retired),
            })
            .await;
        session.shutdown().await;
        Ok(followed?)
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// Republishes the descriptor this device signed in with to the relays
    /// that lack it. Returns how many took it.
    #[wasm_bindgen(js_name = keepDescriptor)]
    pub async fn keep_descriptor(&self) -> Result<u32, CoreError> {
        Ok(account::keep_descriptor(&self.store, &self.keys).await? as u32)
    }

    /// The page wrote to the local database: take it in and send it.
    pub fn written(&self) {
        self.written.notify_one();
    }
}
