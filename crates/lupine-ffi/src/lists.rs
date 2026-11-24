//! Editing the drive's relays and servers, and copying every blob to the
//! servers the drive moves to.

use std::sync::Arc;

use lupine_core::lists;

use crate::CoreError;
use crate::account::{relay_urls, server_urls};
use crate::local::LocalDrive;
use crate::reader::Cancellation;

#[derive(Debug, uniffi::Record)]
pub struct ListsChange {
    pub relays: bool,
    pub servers: bool,
}

/// Publishes the drive's new relays and servers, one address per entry,
/// then applies them to the local database: open the drive again. Servers
/// added must take encrypted chunks.
#[uniffi::export(async_runtime = "tokio")]
pub async fn change_lists(
    drive: Arc<LocalDrive>,
    secret: String,
    relays: Vec<String>,
    servers: Vec<String>,
) -> Result<ListsChange, CoreError> {
    let keys = drive.keys(&secret)?;
    let (relays, servers) = (relay_urls(&relays)?, server_urls(&servers));
    let changed = lists::change(&drive.store, &keys, relays, servers).await?;
    Ok(ListsChange {
        relays: changed.relays,
        servers: changed.servers,
    })
}

#[uniffi::export(callback_interface)]
pub trait FillObserver: Send + Sync {
    fn progressed(&self, done: u64, total: u64);
}

#[derive(Debug, uniffi::Record)]
pub struct FillReport {
    pub blobs: u64,
    /// On no server at all.
    pub lost: u64,
    /// Left for the next fill, with the reason.
    pub failed: Vec<String>,
}

/// Copies every blob the drive references to each of its servers that
/// lacks it. Run after the drive session synced every relay, while
/// [`DriveInfo::filling`](crate::local::DriveInfo) holds, until
/// `cancellation`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn fill_servers(
    drive: Arc<LocalDrive>,
    secret: String,
    observer: Box<dyn FillObserver>,
    cancellation: Arc<Cancellation>,
) -> Result<FillReport, CoreError> {
    let keys = drive.keys(&secret)?;
    let progress = |done: usize, total: usize| observer.progressed(done as u64, total as u64);
    let filled = tokio::select! {
        () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        filled = lists::fill(&drive.store, &keys, progress) => filled?,
    };
    Ok(FillReport {
        blobs: filled.blobs as u64,
        lost: filled.lost.len() as u64,
        failed: filled
            .failed
            .iter()
            .map(|(hash, e)| format!("{hash}: {e}"))
            .collect(),
    })
}
