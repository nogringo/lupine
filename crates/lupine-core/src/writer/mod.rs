//! Changes a file system integration makes from a process of its own, such
//! as a File Provider extension. Uploads content, then writes each version
//! to the local database with the item it shows, under the extension's own
//! device id. The wraps wait in the outbox, which the drive session sends to
//! the relays.
//!
//! The item written is what the integration asked for. When the tree rules
//! show something else, the session corrects the item once it takes the
//! version in.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::{StreamExt, TryStreamExt, stream};
use nostr::event::EventId;
use tokio::sync::Semaphore;
use unicode_normalization::UnicodeNormalization;

use crate::blob::BlobStore;
use crate::content::{self, FileKey, Progress, TransferOptions};
use crate::drive::DriveKeys;
use crate::error::{Error, Result};
use crate::graph::History;
use crate::ids::{NodeId, VersionId};
use crate::io::ReadAt;
use crate::items::{self, Item, ItemChanges};
use crate::names;
use crate::node::{DeviceClock, FileMeta, NodeType, NodeVersion};
use crate::replica::{Record, WrapState};
use crate::store::{Store, Writer};
use crate::task::blocking;
use crate::tree::EntryKind;

/// Chunk buffers of the uploads in flight, in MiB.
const UPLOAD_BUDGET: u32 = 128;
const MIB: u64 = 1 << 20;
const REPLICATE_CONCURRENCY: usize = 8;
const PAGE: usize = 500;

/// Where an item goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// `None` for the drive root.
    pub parent: Option<NodeId>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct NewItem {
    /// The integration's id for this creation. Replayed after a crash, the
    /// creation finds the item it made.
    pub request: String,
    pub place: Place,
    pub folder: bool,
    /// The integration imports an item again: it may match an item of the
    /// same name already in the drive.
    pub may_exist: bool,
    /// Unix milliseconds.
    pub created: Option<i64>,
    pub modified: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Edit {
    /// The [`Item::content_version`] the integration showed when the edit
    /// was made. New content based on another one is a concurrent edit.
    pub base_content: Option<[u8; 32]>,
    /// A move or a rename.
    pub place: Option<Place>,
    /// Unix milliseconds. `None` keeps the current value.
    pub created: Option<i64>,
    pub modified: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Deletion {
    /// Tombstones written, for the item and everything under it.
    Deleted(usize),
    /// The file changed after the content the integration deleted: the
    /// newer content wins over the deletion and shows again.
    Kept(Item),
    /// Gone already.
    Unknown,
}

pub struct ItemWriter<S> {
    keys: DriveKeys,
    epoch: u32,
    store: Arc<Store>,
    blobs: S,
    options: TransferOptions,
    /// One write at a time, so each version is written after seeing the
    /// last, as version vectors take it.
    writing: Mutex<()>,
    uploads: Semaphore,
}

/// The versions one write adds, with the items they change.
struct Batch<'a> {
    keys: &'a DriveKeys,
    epoch: u32,
    clock: DeviceClock,
    records: Vec<Record>,
    outbox: Vec<(EventId, bool)>,
    items: ItemChanges,
}

impl Batch<'_> {
    fn add(&mut self, version: NodeVersion, barrier: bool) -> Result<()> {
        let (id, event) = self.keys.wrap(&version)?;
        self.outbox.push((event.id, barrier));
        self.records.push(Record::Wrap {
            id: event.id,
            created_at: event.created_at,
            state: WrapState::Version(id),
            event: Some(event),
        });
        self.records.push(Record::Version { id, version });
        Ok(())
    }
}

impl<S: BlobStore> ItemWriter<S> {
    pub fn new(store: Arc<Store>, keys: DriveKeys, blobs: S) -> Result<Self> {
        let drive = store
            .drive()?
            .ok_or(Error::Malformed("no drive in the local database"))?;
        if drive.pubkey != keys.public_key() {
            return Err(Error::Malformed("this key does not open the drive"));
        }
        Ok(Self {
            keys,
            epoch: drive.epoch,
            store,
            blobs,
            options: TransferOptions::default(),
            writing: Mutex::new(()),
            uploads: Semaphore::new(UPLOAD_BUDGET as usize),
        })
    }

    /// The chunk size applies to new files; an edit keeps its file's.
    pub fn with_options(mut self, options: TransferOptions) -> Self {
        self.options = options;
        self
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn blobs(&self) -> &S {
        &self.blobs
    }

    /// Creates a file from `contents`, or a folder. `None` when an import
    /// matches no item and brings no content to create one from.
    pub async fn create(
        &self,
        new: NewItem,
        contents: Option<&Path>,
        progress: Progress<'_>,
    ) -> Result<Option<Item>> {
        let name = checked_name(&new.place.name)?;
        let parent = new.place.parent;
        let node = NodeId::created(&self.store.clock(Writer::Extension)?.device, &new.request);
        if let Some(existing) = self.store.item(&node)? {
            return Ok(Some(existing));
        }
        if new.may_exist
            && let Some(existing) = self.store.child_named(parent, &name)?
        {
            if contents.is_none() || existing.is_folder() {
                return Ok(Some(existing));
            }
            let edit = Edit {
                base_content: existing.content_version(),
                created: new.created,
                modified: new.modified,
                ..Edit::default()
            };
            let edited = self
                .modify(existing.node, false, edit, contents, progress)
                .await?;
            return Ok(Some(edited));
        }
        self.check_parent(parent)?;
        let file = match (new.folder, contents) {
            (true, _) => None,
            (false, Some(path)) => Some(self.upload(path, None, progress).await?),
            (false, None) if new.may_exist => return Ok(None),
            (false, None) => return Err(Error::Malformed("a new file needs its content")),
        };

        self.write(|batch| {
            let versions = self.store.node_versions(&node)?;
            let history = history(&versions);
            let replaced = history.heads().map(|h| h.versions()).unwrap_or_default();
            let stamp = batch.clock.stamp(batch.epoch);
            let mut version = match &file {
                Some(file) => NodeVersion::file(
                    stamp,
                    node,
                    parent,
                    &name,
                    &file.file_key(),
                    file.content.clone(),
                ),
                None => NodeVersion::folder(stamp, node, parent, &name),
            };
            version.created = new.created;
            version.modified = new.modified;
            let version = version.replacing(&replaced)?;
            let item = shown(&version, parent, name.clone());
            batch.items.updated.push(item.clone());
            batch.add(version, false)?;
            Ok(Some(item))
        })
    }

    /// Moves, renames or touches `node`, and replaces a file's content with
    /// `contents`. `folder` says what the integration shows, for a node
    /// deleted elsewhere that the edit brings back.
    pub async fn modify(
        &self,
        node: NodeId,
        folder: bool,
        edit: Edit,
        contents: Option<&Path>,
        progress: Progress<'_>,
    ) -> Result<Item> {
        let place = edit
            .place
            .as_ref()
            .map(|place| {
                Ok::<_, Error>(Place {
                    parent: place.parent,
                    name: checked_name(&place.name)?,
                })
            })
            .transpose()?;
        if let Some(place) = &place {
            self.check_parent(place.parent)?;
        }
        let file = match contents {
            Some(path) if !folder => {
                let base = self.base_file(&node)?;
                Some(self.upload(path, base.as_ref(), progress).await?)
            }
            _ => None,
        };

        self.write(|batch| {
            let versions = self.store.node_versions(&node)?;
            let history = history(&versions);
            let heads = history.heads().ok_or(Error::UnknownNode)?;
            let (_, winner) = heads.winner();
            let current = self.store.item(&node)?;
            let mut next = if winner.is_tombstone() {
                revived(&history, winner, folder, file.is_some())?
            } else {
                winner.clone()
            };
            if let Some(place) = &place {
                next.parent = place.parent;
                next.name.clone_from(&place.name);
            }
            if let Some(file) = &file {
                next.file = Some(file.clone());
            }
            if edit.created.is_some() {
                next.created = edit.created;
            }
            if edit.modified.is_some() {
                next.modified = edit.modified;
            }
            let (parent, name) = match (&place, &current) {
                (Some(place), _) => (place.parent, place.name.clone()),
                (None, Some(current)) => (current.parent, current.name.clone()),
                (None, None) => (next.parent, next.name.clone()),
            };
            let item = shown(&next, parent, name);

            let stale =
                file.is_some() && items::content_version(winner.file.as_ref()) != edit.base_content;
            if !stale && next.same_state(winner) {
                return Ok(current.unwrap_or(item));
            }
            // New content made from content this device no longer shows
            // replaces nothing: it stays in conflict with what came since,
            // which becomes a conflict copy.
            let replaced = if stale { Vec::new() } else { heads.seen() };
            let version = next.rewritten(batch.clock.stamp(batch.epoch), &replaced)?;
            batch.items.updated.push(item.clone());
            batch.add(version, false)?;
            Ok(item)
        })
    }

    /// Tombstones for `node` and everything under it, one level at a time
    /// from the deepest, so the session sends no folder's before those of
    /// what it holds. `base_content` is what the integration showed of a
    /// file.
    pub fn delete(
        &self,
        node: NodeId,
        base_content: Option<[u8; 32]>,
        recursive: bool,
    ) -> Result<Deletion> {
        let Some(item) = self.store.item(&node)? else {
            return Ok(Deletion::Unknown);
        };
        if item.file.is_some() && item.content_version() != base_content {
            // A new anchor, so the integration learns of the item again.
            self.write(|batch| {
                batch.items.updated.push(item.clone());
                Ok(())
            })?;
            return Ok(Deletion::Kept(item));
        }
        let levels = self.subtree(node)?;
        if levels.len() > 1 && !recursive {
            return Err(Error::FolderNotEmpty);
        }

        self.write(|batch| {
            let mut written = 0;
            for level in levels.iter().rev() {
                let mut barrier = true;
                for current in level {
                    let versions = self.store.node_versions(current)?;
                    let history = history(&versions);
                    let Some(heads) = history.heads() else {
                        continue;
                    };
                    let (winner_id, winner) = heads.winner();
                    if winner.is_tombstone() {
                        continue;
                    }
                    // A conflicting file content nobody here saw survives
                    // the deletion. Folders hold no content.
                    let mut replaced = vec![winner];
                    replaced.extend(
                        heads
                            .iter()
                            .filter(|(id, head)| {
                                *id != winner_id
                                    && (winner.node_type == NodeType::Folder
                                        || head.same_state(winner))
                            })
                            .map(|(_, head)| head),
                    );
                    let tombstone =
                        NodeVersion::tombstone(batch.clock.stamp(batch.epoch), &replaced)?;
                    batch.add(tombstone, barrier)?;
                    barrier = false;
                    written += 1;
                }
                batch.items.removed.extend(level.iter().copied());
            }
            Ok(Deletion::Deleted(written))
        })
    }

    fn write<T>(&self, change: impl FnOnce(&mut Batch<'_>) -> Result<T>) -> Result<T> {
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let mut batch = Batch {
            keys: &self.keys,
            epoch: self.epoch,
            clock: self.store.clock(Writer::Extension)?,
            records: Vec::new(),
            outbox: Vec::new(),
            items: ItemChanges::default(),
        };
        let value = change(&mut batch)?;
        self.store
            .commit_extension(&batch.records, batch.clock, &batch.items, &batch.outbox)?;
        Ok(value)
    }

    /// A parent the tree shows deleted comes back with what moves in.
    fn check_parent(&self, parent: Option<NodeId>) -> Result<()> {
        let Some(parent) = parent else {
            return Ok(());
        };
        let versions = self.store.node_versions(&parent)?;
        let heads = history(&versions).heads().ok_or(Error::UnknownParent)?;
        match heads.winner().1.node_type {
            NodeType::File => Err(Error::Malformed("the parent is a file")),
            NodeType::Folder | NodeType::Deleted => Ok(()),
        }
    }

    /// The content new content is uploaded against, keeping its key and
    /// the chunks that did not change. Keys are not reused across a key
    /// rotation.
    fn base_file(&self, node: &NodeId) -> Result<Option<FileMeta>> {
        let versions = self.store.node_versions(node)?;
        let Some(heads) = history(&versions).heads() else {
            return Ok(None);
        };
        let (_, winner) = heads.winner();
        Ok(winner.file.clone().filter(|_| winner.epoch == self.epoch))
    }

    async fn upload(
        &self,
        path: &Path,
        base: Option<&FileMeta>,
        progress: Progress<'_>,
    ) -> Result<FileMeta> {
        let path = path.to_owned();
        let source = Arc::new(blocking(move || std::fs::File::open(path)).await?);
        let size = source.size()?;
        let previous = match base {
            Some(file) => {
                let key = file.file_key();
                match file.content.resolve(&self.blobs, &key).await {
                    Ok(previous) => Some((key, previous)),
                    Err(Error::BlobNotFound(_)) => None,
                    Err(e) => return Err(e),
                }
            }
            None => None,
        };
        let options = TransferOptions {
            chunk_size: previous
                .as_ref()
                .map_or(self.options.chunk_size, |(_, p)| p.layout().chunk_size()),
            ..self.options
        };
        let key = previous
            .as_ref()
            .map_or_else(FileKey::generate, |(key, _)| key.clone());
        let previous = previous.map(|(_, content)| content);

        let in_flight = u64::from(options.chunk_size) * options.concurrency as u64;
        let buffers = size
            .min(in_flight)
            .div_ceil(MIB)
            .clamp(1, UPLOAD_BUDGET.into()) as u32;
        let _buffers = self
            .uploads
            .acquire_many(buffers)
            .await
            .expect("the upload budget is never closed");
        let uploaded = content::upload(
            &self.blobs,
            source,
            &key,
            previous.as_ref(),
            options,
            progress,
        )
        .await?;
        if let Some(previous) = &previous {
            let kept: HashSet<_> = previous.chunks().iter().map(|c| c.blob).collect();
            let reused: Vec<_> = uploaded
                .chunks()
                .iter()
                .map(|c| c.blob)
                .filter(|blob| kept.contains(blob))
                .collect();
            stream::iter(reused)
                .map(|blob| self.blobs.replicate(blob))
                .buffer_unordered(REPLICATE_CONCURRENCY)
                .try_collect::<()>()
                .await?;
        }
        let content = uploaded.describe(&self.blobs, &key).await?;
        Ok(FileMeta {
            key: key.root().clone(),
            content,
        })
    }

    /// The items under `node` level by level, `node` first.
    fn subtree(&self, node: NodeId) -> Result<Vec<Vec<NodeId>>> {
        let mut seen = HashSet::from([node]);
        let mut levels = vec![vec![node]];
        loop {
            let mut next = Vec::new();
            for parent in levels.last().expect("a level") {
                let mut after = None;
                loop {
                    let page = self.store.children(Some(*parent), after, PAGE)?;
                    after = page.last().map(|item| item.node);
                    let more = page.len() == PAGE;
                    next.extend(
                        page.into_iter()
                            .map(|item| item.node)
                            .filter(|child| seen.insert(*child)),
                    );
                    if !more {
                        break;
                    }
                }
            }
            if next.is_empty() {
                return Ok(levels);
            }
            levels.push(next);
        }
    }
}

fn history(versions: &[(VersionId, NodeVersion)]) -> History<'_> {
    History::new(versions.iter().map(|(id, version)| (*id, version)))
}

fn checked_name(name: &str) -> Result<String> {
    let name: String = name.nfc().collect();
    names::validate(&name)?;
    Ok(name)
}

/// A node deleted elsewhere, as the edit brings it back: its last live
/// state, at the place its tombstone kept.
fn revived(
    history: &History<'_>,
    tombstone: &NodeVersion,
    folder: bool,
    has_content: bool,
) -> Result<NodeVersion> {
    let live = history
        .versions()
        .iter()
        .map(|(_, version)| *version)
        .filter(|version| !version.is_tombstone())
        .max_by_key(|version| version.time);
    let mut next = match live {
        Some(live) => live.clone(),
        None if folder || has_content => {
            let mut next = tombstone.clone();
            next.node_type = if folder {
                NodeType::Folder
            } else {
                NodeType::File
            };
            next
        }
        None => return Err(Error::UnknownNode),
    };
    next.parent = tombstone.parent;
    next.name.clone_from(&tombstone.name);
    Ok(next)
}

fn shown(version: &NodeVersion, parent: Option<NodeId>, name: String) -> Item {
    Item {
        node: version.node,
        parent,
        name,
        kind: match version.node_type {
            NodeType::File => EntryKind::File,
            NodeType::Folder | NodeType::Deleted => EntryKind::Folder,
        },
        created: version.created,
        modified: version.modified,
        file: version.file.clone(),
    }
}

#[cfg(test)]
mod tests;
