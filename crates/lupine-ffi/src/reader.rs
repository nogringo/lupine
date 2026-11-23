//! File content for the File Provider extension, whole or in part.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lupine_core::blossom::BlossomStore;
use lupine_core::fetch::FileReader;
use lupine_core::store::Store;
use tokio::sync::Notify;

use crate::CoreError;
use crate::local::{DriveItem, LocalDrive, node};

/// Swift tasks do not cancel Rust futures, so calls that may need
/// cancelling take one of these.
#[derive(Default, uniffi::Object)]
pub struct Cancellation {
    cancelled: AtomicBool,
    notify: Notify,
}

#[uniffi::export]
impl Cancellation {
    #[uniffi::constructor]
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

#[uniffi::export(callback_interface)]
pub trait TransferObserver: Send + Sync {
    /// Plaintext bytes transferred so far.
    fn progressed(&self, bytes: u64);
}

#[derive(Debug, uniffi::Record)]
pub struct FetchRequest {
    pub id: String,
    /// Where to write the content, at its offsets.
    pub path: String,
    pub start: u64,
    pub end: u64,
    /// A power of two the returned range must align to, except at the end
    /// of the file.
    pub alignment: u64,
    /// Fail with `VersionNoLongerAvailable` unless the item has this content
    /// version.
    pub content_version: Option<Vec<u8>>,
}

#[derive(Debug, uniffi::Record)]
pub struct Fetched {
    /// The item as fetched.
    pub item: DriveItem,
    pub start: u64,
    pub end: u64,
}

#[derive(uniffi::Object)]
pub struct ContentReader {
    store: Arc<Store>,
    reader: FileReader<BlossomStore>,
}

#[uniffi::export]
pub fn open_content_reader(
    drive: Arc<LocalDrive>,
    secret: String,
) -> Result<Arc<ContentReader>, CoreError> {
    let keys = drive.keys(&secret)?;
    let blobs = BlossomStore::new(&drive.record()?.servers, keys.keys())?;
    Ok(Arc::new(ContentReader {
        store: Arc::clone(&drive.store),
        reader: FileReader::new(blobs),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl ContentReader {
    pub async fn fetch(
        &self,
        request: FetchRequest,
        observer: Box<dyn TransferObserver>,
        cancellation: Arc<Cancellation>,
    ) -> Result<Fetched, CoreError> {
        let item = self
            .store
            .item(&node(&request.id)?)?
            .ok_or(CoreError::NoSuchItem)?;
        let file = item
            .file
            .as_ref()
            .ok_or_else(|| CoreError::Check("a folder has no content".into()))?;
        if let Some(expected) = &request.content_version
            && item.content_version().as_ref().map(<[u8; 32]>::as_slice)
                != Some(expected.as_slice())
        {
            return Err(CoreError::VersionNoLongerAvailable);
        }
        let progress = |bytes| observer.progressed(bytes);
        let read = self.reader.read(
            file,
            request.start..request.end,
            request.alignment,
            Path::new(&request.path),
            &progress,
        );
        let range = tokio::select! {
            range = read => range?,
            () = cancellation.cancelled() => return Err(CoreError::Cancelled),
        };
        Ok(Fetched {
            item: item.into(),
            start: range.start,
            end: range.end,
        })
    }
}
