//! Swift and Kotlin facade over `lupine-core`.

use std::sync::Arc;
use std::time::Instant;

use lupine_core::blob::MemoryBlobStore;
use lupine_core::content::{self, FileKey, TransferOptions};
use lupine_core::io::MemoryFile;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CoreError {
    #[error("{0}")]
    Core(#[from] lupine_core::Error),
    #[error("{0}")]
    Check(String),
}

#[derive(Debug, uniffi::Record)]
pub struct SelfTestReport {
    pub bytes: u64,
    pub blobs: u64,
    pub millis: u64,
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Encrypts `size` bytes through the content pipeline into memory and reads
/// them back, so an app can check that the core and its runtime work.
#[uniffi::export(async_runtime = "tokio")]
pub async fn self_test(size: u64) -> Result<SelfTestReport, CoreError> {
    let started = Instant::now();
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
    Ok(SelfTestReport {
        bytes: size,
        blobs: store.len() as u64,
        millis: started.elapsed().as_millis() as u64,
    })
}
