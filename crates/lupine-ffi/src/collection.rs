//! Deleting from the drive's servers the blobs no version references any
//! more.

use std::sync::Arc;

use lupine_core::collection;

use crate::CoreError;
use crate::local::LocalDrive;
use crate::reader::Cancellation;

#[derive(Debug, uniffi::Record)]
pub struct CollectionReport {
    /// Deleted from every drive server.
    pub deleted: u64,
    /// Nothing was deleted: the drive holds a version this device cannot
    /// read, which may reference any blob.
    pub held_back: bool,
    /// Left for the next collection, with the reason.
    pub failed: Vec<String>,
}

/// Deletes from the drive's servers the blobs that no version has
/// referenced for 30 days. Run right after the drive session synced every
/// relay, until `cancellation`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn collect_blobs(
    drive: Arc<LocalDrive>,
    secret: String,
    cancellation: Arc<Cancellation>,
) -> Result<CollectionReport, CoreError> {
    let keys = drive.keys(&secret)?;
    let collected = tokio::select! {
        () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        collected = collection::collect(&drive.store, &keys) => collected?,
    };
    Ok(CollectionReport {
        deleted: collected.deleted as u64,
        held_back: collected.held_back,
        failed: collected
            .failed
            .iter()
            .map(|(hash, e)| format!("{hash}: {e}"))
            .collect(),
    })
}
