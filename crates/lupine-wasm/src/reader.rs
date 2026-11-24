//! File content, downloaded into OPFS files.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use js_sys::Function;
use lupine_core::blossom::BlossomStore;
use lupine_core::fetch::FileReader;
use lupine_core::store::Store;
use tokio::sync::Notify;
use wasm_bindgen::prelude::*;
use web_sys::FileSystemSyncAccessHandle;

use crate::io::HandleSink;
use crate::local::{DriveItem, LocalDrive, node};
use crate::{CoreError, to_js};

/// Calls that may need cancelling take one of these.
#[wasm_bindgen]
#[derive(Default)]
pub struct Cancellation {
    cancelled: AtomicBool,
    notify: Notify,
}

#[wasm_bindgen]
impl Cancellation {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

impl Cancellation {
    pub(crate) async fn cancelled(&self) {
        let notified = self.notify.notified();
        if !self.cancelled.load(Ordering::SeqCst) {
            notified.await;
        }
    }
}

/// `progress(bytes)` gets the plaintext bytes transferred so far.
pub(crate) fn progress(observer: &Function) -> impl Fn(u64) + '_ {
    move |bytes| {
        let _ = observer.call1(&JsValue::NULL, &JsValue::from_f64(bytes as f64));
    }
}

#[wasm_bindgen]
pub struct ContentReader {
    store: Arc<Store>,
    reader: FileReader<BlossomStore>,
}

#[wasm_bindgen(js_name = openContentReader)]
pub fn open_content_reader(drive: &LocalDrive, secret: String) -> Result<ContentReader, CoreError> {
    let keys = drive.keys(&secret)?;
    Ok(ContentReader {
        store: Arc::clone(&drive.store),
        reader: FileReader::new(drive.blobs(&keys)?),
    })
}

#[wasm_bindgen]
impl ContentReader {
    /// Writes the whole content of item `id` into `sink`, and returns the
    /// item as fetched. `content_version` fails the read with
    /// `VersionNoLongerAvailable` when the item no longer has it.
    #[wasm_bindgen(unchecked_return_type = "DriveItem")]
    pub async fn fetch(
        &self,
        id: String,
        content_version: Option<String>,
        sink: FileSystemSyncAccessHandle,
        #[wasm_bindgen(unchecked_param_type = "(bytes: number) => void")] observer: Function,
        cancellation: &Cancellation,
    ) -> Result<JsValue, CoreError> {
        let item = self.store.item(&node(&id)?)?.ok_or(CoreError::NoSuchItem)?;
        let file = item
            .file
            .as_ref()
            .ok_or_else(|| CoreError::Check("a folder has no content".into()))?;
        if let Some(expected) = content_version
            && item.content_version().map(hex::encode) != Some(expected)
        {
            return Err(CoreError::VersionNoLongerAvailable);
        }
        let progress = progress(&observer);
        let read = self.reader.read_into(
            file,
            0..file.content.size,
            1,
            Arc::new(HandleSink::new(sink)),
            &progress,
        );
        tokio::select! {
            read = read => read?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        to_js(&DriveItem::from(item))
    }
}
