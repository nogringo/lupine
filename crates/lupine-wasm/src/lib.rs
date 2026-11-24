//! The Lupine core for the web app, which runs it in a dedicated worker: the
//! worker owns the local database, in OPFS, and every connection to relays
//! and servers. Mirrors `lupine-ffi`.

#![cfg(target_family = "wasm")]

// The `Send` and `Sync` impls of browser objects in this crate rely on it.
#[cfg(target_feature = "atomics")]
compile_error!("lupine-wasm assumes a single thread");

use std::sync::Arc;

use lupine_core::blob::MemoryBlobStore;
use lupine_core::content::{self, FileKey, TransferOptions};
use lupine_core::io::MemoryFile;
use serde::Serialize;
use wasm_bindgen::prelude::*;

mod account;
mod clock;
mod collection;
mod error;
mod io;
mod lists;
mod local;
mod reader;
mod signer;
mod storage;
mod sync;
mod writer;

pub use error::CoreError;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen(js_name = coreVersion)]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[derive(Serialize)]
pub struct SelfTestReport {
    pub bytes: u64,
    pub blobs: u64,
    pub millis: u64,
}

#[wasm_bindgen(typescript_custom_section)]
const SELF_TEST_REPORT: &str = r#"
export interface SelfTestReport {
    bytes: number;
    blobs: number;
    millis: number;
}
"#;

/// Encrypts `size` bytes through the content pipeline into memory and reads
/// them back, so the app can check that the core works in this browser.
#[wasm_bindgen(js_name = selfTest, unchecked_return_type = "SelfTestReport")]
pub async fn self_test(size: u32) -> Result<JsValue, CoreError> {
    let started = web_time::Instant::now();
    let size = u64::from(size);
    let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let options = TransferOptions::default();

    let source = Arc::new(MemoryFile::new(bytes.clone()));
    let uploaded = content::upload(&store, source, &key, None, options, &|_| {}).await?;
    let sink = Arc::new(MemoryFile::default());
    content::download(
        &store,
        &uploaded,
        &key,
        0..size,
        Arc::clone(&sink),
        options,
        &|_| {},
    )
    .await?;

    if sink.snapshot() != bytes {
        return Err(CoreError::Check("decrypted bytes differ".into()));
    }
    to_js(&SelfTestReport {
        bytes: size,
        blobs: store.len() as u64,
        millis: started.elapsed().as_millis() as u64,
    })
}

/// Plain objects, so they cross to the page with `postMessage`.
pub(crate) fn to_js<T: Serialize>(value: &T) -> Result<JsValue, CoreError> {
    let serializer = serde_wasm_bindgen::Serializer::json_compatible();
    value
        .serialize(&serializer)
        .map_err(|e| CoreError::Core(e.to_string()))
}
