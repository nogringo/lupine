//! Two-way sync between a folder of this device and a folder of the drive.
//! Each pass merges three trees: the local folder as scanned, the drive
//! folder as the local database shows it, and what both were when they last
//! agreed. What changed on one side is applied to the other. What changed on
//! both is settled the way the drive settles concurrent edits: an edit wins
//! over a deletion, the drive's place wins over a local move, and differing
//! contents end in a conflict copy.
//!
//! It writes versions as a File Provider extension does, under a device id
//! of its own, and leaves their wraps in the outbox for the session to send.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use unicode_normalization::UnicodeNormalization;

use crate::blob::BlobStore;
use crate::content::TransferOptions;
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::fetch::FileReader;
use crate::ids::NodeId;
use crate::names::{self, Folded};
use crate::store::{Store, Writer};
use crate::task::{self, blocking};
use crate::writer::{ItemWriter, NewItem, Place};

mod fs;
mod pass;
mod scan;
#[cfg(test)]
mod tests;

/// Passes run again while they change something, since what one side
/// shows may only settle once the other took the change in.
const PASSES: usize = 3;
/// A pass even when nothing signals a change, for what watchers miss.
pub const RESCAN: Duration = Duration::from_secs(5 * 60);
/// Changes that arrive this close together are synced together.
const DEBOUNCE: Duration = Duration::from_secs(1);
/// Where a pass stages downloads and snapshots, inside the folder so that
/// renames stay on its volume. Never synced.
const STAGING: &str = ".lupine-sync";

/// Never synced: Finder's own files, and the staging folder.
pub(crate) fn ignored(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Icon\r" | STAGING)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncedFolder {
    pub id: i64,
    /// The drive folder it syncs with.
    pub root: NodeId,
    pub path: PathBuf,
    /// Another folder found at `path`, such as the empty mount point of a
    /// volume that is not mounted, is not this one.
    pub file_id: u64,
    /// The shell's own reference to the folder, such as a security-scoped
    /// bookmark, opaque to the core.
    pub bookmark: Option<Vec<u8>>,
}

/// What a pass compares to tell whether a local entry changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    /// Survives renames, so moves are told from deletions.
    pub file_id: u64,
    pub size: u64,
    /// Unix nanoseconds.
    pub mtime: i64,
}

/// An entry as both sides last agreed on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Synced {
    pub node: NodeId,
    /// The folder's root node at its top.
    pub parent: NodeId,
    pub name: String,
    pub folder: bool,
    /// The [`Item::content_version`](crate::items::Item::content_version)
    /// of the drive's file.
    pub content: Option<[u8; 32]>,
    /// The drive's modification time, Unix milliseconds.
    pub modified: Option<i64>,
    /// Only `file_id` counts for folders.
    pub local: Stat,
}

/// Where entries deleted from the drive go, so a deletion made elsewhere
/// can still be undone on this device.
pub trait Trash: Send + Sync {
    fn trash(&self, path: &Path) -> io::Result<()>;
}

/// Deletes for good, where there is no trash.
pub struct Delete;

impl Trash for Delete {
    fn trash(&self, path: &Path) -> io::Result<()> {
        if std::fs::symlink_metadata(path)?.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }
}

#[derive(Debug, Default)]
pub struct FolderReport {
    /// Versions written: the session should send them.
    pub written: usize,
    /// Changes made to the local folder.
    pub applied: usize,
    pub uploaded: usize,
    pub downloaded: usize,
    /// Entries left for the next pass, by path in the folder, with why.
    pub failed: Vec<(String, Error)>,
}

impl FolderReport {
    pub fn changed(&self) -> bool {
        self.written > 0 || self.applied > 0
    }
}

/// What [`FolderSync::run`] reports.
#[derive(Debug)]
pub enum FolderEvent<'a> {
    /// Until the session has synced every drive relay once: the drive
    /// folders may still be incomplete.
    Waiting,
    Syncing(&'a SyncedFolder),
    Synced(&'a SyncedFolder, &'a FolderReport),
    /// The folder or its drive folder is missing; it waits for them.
    Failed(&'a SyncedFolder, &'a Error),
}

pub struct FolderSync<S> {
    store: Arc<Store>,
    writer: ItemWriter<Arc<S>>,
    reader: FileReader<Arc<S>>,
    trash: Box<dyn Trash>,
}

impl<S: BlobStore + 'static> FolderSync<S> {
    pub fn new(
        store: Arc<Store>,
        keys: DriveKeys,
        blobs: S,
        trash: Box<dyn Trash>,
    ) -> Result<Self> {
        let blobs = Arc::new(blobs);
        let writer = ItemWriter::new(Arc::clone(&store), keys, Arc::clone(&blobs))?
            .writing_as(Writer::Folders);
        Ok(Self {
            store,
            writer,
            reader: FileReader::new(blobs),
            trash,
        })
    }

    /// The chunk size applies to new files.
    pub fn with_options(mut self, options: TransferOptions) -> Self {
        self.writer = self.writer.with_options(options);
        self
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn folders(&self) -> Result<Vec<SyncedFolder>> {
        self.store.folders()
    }

    /// Syncs the folder at `path` with the drive folder of the same name at
    /// the drive root, created when there is none. Folders may not nest.
    pub async fn add(&self, path: &Path, bookmark: Option<Vec<u8>>) -> Result<SyncedFolder> {
        let (path, file_id) = canonical(path).await?;
        let folders = self.store.folders()?;
        if folders
            .iter()
            .any(|f| path.starts_with(&f.path) || f.path.starts_with(&path))
        {
            return Err(Error::FolderOverlap(
                "this folder is inside a synced folder, or holds one",
            ));
        }
        let name: String = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(Error::InvalidName)?
            .nfc()
            .collect();
        names::validate(&name)?;

        let at_root = self.store.all_children(None)?;
        let existing = at_root.iter().find(|item| {
            item.is_folder()
                && Folded(item.name.clone()) == Folded(name.clone())
                && !folders.iter().any(|f| f.root == item.node)
        });
        let root = match existing {
            Some(item) => item.node,
            None => {
                let taken: BTreeSet<Folded> = at_root
                    .iter()
                    .map(|item| Folded(item.name.clone()))
                    .collect();
                let name = (1..)
                    .map(|n| {
                        if n == 1 {
                            name.clone()
                        } else {
                            names::numbered(&name, n)
                        }
                    })
                    .find(|candidate| !taken.contains(&Folded(candidate.clone())))
                    .expect("a folder holds finitely many names");
                let new = NewItem {
                    request: format!("folder sync {}", NodeId::random()),
                    place: Place { parent: None, name },
                    folder: true,
                    may_exist: false,
                    created: None,
                    modified: None,
                };
                let created = self.writer.create(new, None, &|_| {}).await?;
                created.ok_or(Error::Malformed("no folder created"))?.node
            }
        };
        self.store
            .add_folder(root, &path, file_id, bookmark.as_deref())
    }

    /// Stops syncing: the local folder and the drive folder both stay.
    pub fn remove(&self, id: i64) -> Result<()> {
        self.store.remove_folder(id)
    }

    /// The folder moved, or its bookmark was renewed.
    pub async fn relocate(&self, id: i64, path: &Path, bookmark: Option<Vec<u8>>) -> Result<()> {
        let (path, file_id) = canonical(path).await?;
        self.store
            .relocate_folder(id, &path, file_id, bookmark.as_deref())
    }

    /// One pass over `folder`. Run it only once the session has synced
    /// every drive relay: until then the drive folder may lack entries that
    /// this pass would take for deleted.
    pub async fn sync(&self, folder: &SyncedFolder) -> Result<FolderReport> {
        pass::run(self, folder).await
    }

    /// Syncs every folder until `stop` completes: at the start, whenever
    /// `changed` is notified (a watcher saw the folder change, or the drive
    /// did), and every [`RESCAN`]. Fails only when the local database does.
    pub async fn run(
        &self,
        stop: impl Future<Output = ()>,
        changed: &Notify,
        mut report: impl FnMut(FolderEvent<'_>),
    ) -> Result<()> {
        let mut stop = pin!(stop);
        loop {
            let (_, synced) = self.store.device_state()?;
            if synced {
                for folder in self.store.folders()? {
                    tokio::select! {
                        () = &mut stop => return Ok(()),
                        done = self.passes(&folder, &mut report) => done?,
                    }
                }
            } else {
                report(FolderEvent::Waiting);
            }
            tokio::select! {
                () = &mut stop => return Ok(()),
                () = changed.notified() => {}
                () = task::sleep(RESCAN) => {}
            }
            tokio::select! {
                () = &mut stop => return Ok(()),
                () = task::sleep(DEBOUNCE) => {}
            }
        }
    }

    async fn passes(
        &self,
        folder: &SyncedFolder,
        report: &mut impl FnMut(FolderEvent<'_>),
    ) -> Result<()> {
        for _ in 0..PASSES {
            report(FolderEvent::Syncing(folder));
            match self.sync(folder).await {
                Ok(done) => {
                    report(FolderEvent::Synced(folder, &done));
                    if !done.changed() {
                        break;
                    }
                }
                Err(e @ Error::Database(_)) => return Err(e),
                Err(e) => {
                    report(FolderEvent::Failed(folder, &e));
                    break;
                }
            }
        }
        Ok(())
    }
}

/// The path without links, and the folder's file id.
async fn canonical(path: &Path) -> Result<(PathBuf, u64)> {
    let path = path.to_owned();
    blocking(move || {
        let path = std::fs::canonicalize(path).map_err(|_| Error::LocalFolderMissing)?;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => Ok((path, scan::stat(&meta).file_id)),
            _ => Err(Error::LocalFolderMissing),
        }
    })
    .await
}
