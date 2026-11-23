//! The app's side: keeping the local database up to date with the relays,
//! and sending what the File Provider extension wrote.

use std::sync::Arc;

use lupine_core::session::{DriveSession, Followed};
use tokio::sync::{Mutex, Notify};

use crate::CoreError;
use crate::local::LocalDrive;
use crate::reader::Cancellation;

#[uniffi::export(callback_interface)]
pub trait SyncObserver: Send + Sync {
    fn syncing(&self);
    /// `failed` holds each relay that could not be synced, with the reason.
    fn synced(&self, failed: Vec<String>);
    /// The File Provider extension should enumerate changes.
    fn items_changed(&self);
}

#[derive(uniffi::Object)]
pub struct DriveSync {
    session: Mutex<DriveSession>,
    stop: Cancellation,
    written: Notify,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn open_drive_sync(
    drive: Arc<LocalDrive>,
    secret: String,
) -> Result<Arc<DriveSync>, CoreError> {
    let keys = drive.keys(&secret)?;
    let session = DriveSession::open(Arc::clone(&drive.store), keys).await?;
    Ok(Arc::new(DriveSync {
        session: Mutex::new(session),
        stop: Cancellation::default(),
        written: Notify::new(),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl DriveSync {
    /// Keeps the local database up to date until [`stop`](Self::stop).
    pub async fn run(&self, observer: Box<dyn SyncObserver>) -> Result<(), CoreError> {
        let mut session = self.session.lock().await;
        let followed = session
            .follow(self.stop.cancelled(), &self.written, |event| match event {
                Followed::Syncing => observer.syncing(),
                Followed::Synced(report) => observer.synced(
                    report
                        .failed
                        .iter()
                        .map(|(relay, e)| format!("{relay}: {e}"))
                        .collect(),
                ),
                Followed::ItemsChanged => observer.items_changed(),
            })
            .await;
        session.shutdown().await;
        Ok(followed?)
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// The File Provider extension wrote to the local database: take it in
    /// and send it.
    pub fn written(&self) {
        self.written.notify_one();
    }
}
