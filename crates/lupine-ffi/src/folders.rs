//! Folder sync, in the app: folders of the Mac kept in sync with drive
//! folders. The app watches them and says when they change.

use std::io;
use std::path::Path;
use std::sync::Arc;

use lupine_core::blossom::BlossomStore;
use lupine_core::folder_sync::{self, FolderEvent, Trash};
use tokio::sync::Notify;

use crate::CoreError;
use crate::local::LocalDrive;
use crate::reader::Cancellation;

#[derive(Debug, uniffi::Record)]
pub struct SyncedFolder {
    pub id: i64,
    pub path: String,
    /// What the app gave when adding the folder, such as a security-scoped
    /// bookmark.
    pub bookmark: Option<Vec<u8>>,
}

impl From<folder_sync::SyncedFolder> for SyncedFolder {
    fn from(folder: folder_sync::SyncedFolder) -> Self {
        Self {
            id: folder.id,
            path: folder.path.to_string_lossy().into_owned(),
            bookmark: folder.bookmark,
        }
    }
}

/// Where entries deleted from the drive go on the Mac.
#[uniffi::export(callback_interface)]
pub trait LocalTrash: Send + Sync {
    /// Moves the file or folder at `path` to the Trash. `false` when it
    /// could not.
    fn trash(&self, path: String) -> bool;
}

struct ForeignTrash(Box<dyn LocalTrash>);

impl Trash for ForeignTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        match self.0.trash(path.to_string_lossy().into_owned()) {
            true => Ok(()),
            false => Err(io::Error::other("could not move it to the Trash")),
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait FolderObserver: Send + Sync {
    /// The drive session has yet to sync every relay once.
    fn waiting(&self);
    fn syncing(&self, folder: i64);
    /// `wrote` when the pass changed the drive: the session should send it
    /// and Finder look. `failed` holds the entries left for the next pass,
    /// with the reason.
    fn synced(&self, folder: i64, wrote: bool, failed: Vec<String>);
    /// The folder or its drive folder is missing.
    fn failed(&self, folder: i64, reason: String);
}

#[derive(uniffi::Object)]
pub struct FolderSync {
    sync: folder_sync::FolderSync<BlossomStore>,
    stop: Cancellation,
    changed: Notify,
}

#[uniffi::export]
pub fn open_folder_sync(
    drive: Arc<LocalDrive>,
    secret: String,
    trash: Box<dyn LocalTrash>,
) -> Result<Arc<FolderSync>, CoreError> {
    let keys = drive.keys(&secret)?;
    let blobs = drive.blobs(&keys)?;
    let trash = Box::new(ForeignTrash(trash));
    let sync = folder_sync::FolderSync::new(Arc::clone(&drive.store), keys, blobs, trash)?;
    Ok(Arc::new(FolderSync {
        sync,
        stop: Cancellation::default(),
        changed: Notify::new(),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl FolderSync {
    pub fn folders(&self) -> Result<Vec<SyncedFolder>, CoreError> {
        let folders = self.sync.folders()?;
        Ok(folders.into_iter().map(SyncedFolder::from).collect())
    }

    /// Syncs the folder at `path` with the drive folder of the same name at
    /// the drive root, made when there is none: the session should then
    /// send.
    pub async fn add_folder(
        &self,
        path: String,
        bookmark: Option<Vec<u8>>,
    ) -> Result<SyncedFolder, CoreError> {
        let folder = self.sync.add(Path::new(&path), bookmark).await?;
        self.changed.notify_one();
        Ok(folder.into())
    }

    /// Both the folder and the drive folder stay.
    pub fn remove_folder(&self, id: i64) -> Result<(), CoreError> {
        Ok(self.sync.remove(id)?)
    }

    /// The folder moved, or its bookmark was renewed.
    pub async fn relocate_folder(
        &self,
        id: i64,
        path: String,
        bookmark: Option<Vec<u8>>,
    ) -> Result<(), CoreError> {
        self.sync.relocate(id, Path::new(&path), bookmark).await?;
        self.changed.notify_one();
        Ok(())
    }

    /// Syncs every folder until [`stop`](Self::stop).
    pub async fn run(&self, observer: Box<dyn FolderObserver>) -> Result<(), CoreError> {
        let ran = self
            .sync
            .run(self.stop.cancelled(), &self.changed, |event| match event {
                FolderEvent::Waiting => observer.waiting(),
                FolderEvent::Syncing(folder) => observer.syncing(folder.id),
                FolderEvent::Synced(folder, report) => observer.synced(
                    folder.id,
                    report.written > 0,
                    report
                        .failed
                        .iter()
                        .map(|(path, e)| format!("{path}: {e}"))
                        .collect(),
                ),
                FolderEvent::Failed(folder, e) => observer.failed(folder.id, e.to_string()),
            })
            .await;
        Ok(ran?)
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// A folder changed on the Mac, or the drive changed.
    pub fn changed(&self) {
        self.changed.notify_one();
    }
}
