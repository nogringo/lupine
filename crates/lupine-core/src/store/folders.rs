//! What folder sync keeps: the synced folders, and what each of their
//! entries was when both sides last agreed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rusqlite::{Row, params};

use super::{ITEM_COLUMNS, Store, array, collect_items};
use crate::error::{Error, Result};
use crate::folder_sync::{Stat, Synced, SyncedFolder};
use crate::ids::NodeId;
use crate::items::Item;

const SYNCED_COLUMNS: &str =
    "node, parent, name, is_folder, content, modified, file_id, size, mtime";

impl Store {
    pub fn folders(&self) -> Result<Vec<SyncedFolder>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare_cached("SELECT id, root, path, file_id, bookmark FROM folders ORDER BY id")?;
        let mut rows = statement.query([])?;
        let mut folders = Vec::new();
        while let Some(row) = rows.next()? {
            folders.push(read_folder(row)?);
        }
        Ok(folders)
    }

    pub fn folder(&self, id: i64) -> Result<Option<SyncedFolder>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT id, root, path, file_id, bookmark FROM folders WHERE id = ?1",
        )?;
        let mut rows = statement.query([id])?;
        rows.next()?.map(read_folder).transpose()
    }

    pub(crate) fn add_folder(
        &self,
        root: NodeId,
        path: &Path,
        file_id: u64,
        bookmark: Option<&[u8]>,
    ) -> Result<SyncedFolder> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO folders (root, path, file_id, bookmark) VALUES (?1, ?2, ?3, ?4)",
            params![root.as_bytes(), path_text(path)?, file_id as i64, bookmark],
        )?;
        Ok(SyncedFolder {
            id: conn.last_insert_rowid(),
            root,
            path: path.to_owned(),
            file_id,
            bookmark: bookmark.map(<[u8]>::to_vec),
        })
    }

    pub fn remove_folder(&self, id: i64) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM synced WHERE folder = ?1", [id])?;
        tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn relocate_folder(
        &self,
        id: i64,
        path: &Path,
        file_id: u64,
        bookmark: Option<&[u8]>,
    ) -> Result<()> {
        self.lock().execute(
            "UPDATE folders SET path = ?2, file_id = ?3, bookmark = ?4 WHERE id = ?1",
            params![id, path_text(path)?, file_id as i64, bookmark],
        )?;
        Ok(())
    }

    pub(crate) fn synced(&self, folder: i64) -> Result<Vec<Synced>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {SYNCED_COLUMNS} FROM synced WHERE folder = ?1"
        ))?;
        let mut rows = statement.query([folder])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(read_synced(row)?);
        }
        Ok(entries)
    }

    /// Writes nothing once the folder is no longer synced.
    pub(crate) fn record_synced(&self, folder: i64, entries: &[Synced]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut upsert = tx.prepare_cached(&format!(
                "INSERT OR REPLACE INTO synced (folder, {SYNCED_COLUMNS})
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                 WHERE EXISTS (SELECT 1 FROM folders WHERE id = ?1)"
            ))?;
            for entry in entries {
                upsert.execute(params![
                    folder,
                    entry.node.as_bytes(),
                    entry.parent.as_bytes(),
                    entry.name,
                    entry.folder,
                    entry.content.as_ref().map(<[u8; 32]>::as_slice),
                    entry.modified,
                    entry.local.file_id as i64,
                    entry.local.size as i64,
                    entry.local.mtime,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn forget_synced(&self, folder: i64, nodes: &[NodeId]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut delete =
                tx.prepare_cached("DELETE FROM synced WHERE folder = ?1 AND node = ?2")?;
            for node in nodes {
                delete.execute(params![folder, node.as_bytes()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every item in `parent`, `None` for the drive root.
    pub(crate) fn all_children(&self, parent: Option<NodeId>) -> Result<Vec<Item>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items WHERE parent IS ?1 AND removed = 0"
        ))?;
        collect_items(statement.query([parent.as_ref().map(NodeId::as_bytes)])?)
    }

    /// Every item under `root`, parents before their children, read in one
    /// transaction so that the tree is consistent.
    pub(crate) fn subtree(&self, root: NodeId) -> Result<Vec<Item>> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut items = Vec::new();
        {
            let mut statement = tx.prepare_cached(&format!(
                "SELECT {ITEM_COLUMNS} FROM items WHERE parent = ?1 AND removed = 0"
            ))?;
            let mut pending = VecDeque::from([root]);
            while let Some(parent) = pending.pop_front() {
                let children = collect_items(statement.query([parent.as_bytes()])?)?;
                pending.extend(
                    children
                        .iter()
                        .filter(|item| item.is_folder())
                        .map(|item| item.node),
                );
                items.extend(children);
            }
        }
        tx.commit()?;
        Ok(items)
    }
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or(Error::Malformed("the folder path is not valid UTF-8"))
}

fn read_folder(row: &Row<'_>) -> Result<SyncedFolder> {
    Ok(SyncedFolder {
        id: row.get(0)?,
        root: NodeId(array(row.get(1)?)?),
        path: PathBuf::from(row.get::<_, String>(2)?),
        file_id: row.get::<_, i64>(3)? as u64,
        bookmark: row.get(4)?,
    })
}

fn read_synced(row: &Row<'_>) -> Result<Synced> {
    let content: Option<Vec<u8>> = row.get(4)?;
    Ok(Synced {
        node: NodeId(array(row.get(0)?)?),
        parent: NodeId(array(row.get(1)?)?),
        name: row.get(2)?,
        folder: row.get(3)?,
        content: content.map(array).transpose()?,
        modified: row.get(5)?,
        local: Stat {
            file_id: row.get::<_, i64>(6)? as u64,
            size: row.get::<_, i64>(7)? as u64,
            mtime: row.get(8)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DriveId;
    use crate::store::DriveRecord;

    #[test]
    fn synced_entries_go_with_their_folder() {
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: DriveId([1; 16]),
                pubkey: nostr::key::Keys::generate().public_key(),
                epoch: 0,
                relays: Vec::new(),
                servers: Vec::new(),
            })
            .unwrap();
        let folder = store
            .add_folder(
                NodeId::random(),
                Path::new("/Users/me/Notes"),
                9,
                Some(b"mark"),
            )
            .unwrap();
        assert_eq!(store.folders().unwrap(), std::slice::from_ref(&folder));
        let entry = Synced {
            node: NodeId::random(),
            parent: folder.root,
            name: "a.txt".into(),
            folder: false,
            content: Some([4; 32]),
            modified: Some(5),
            local: Stat {
                file_id: u64::MAX,
                size: 3,
                mtime: -7,
            },
        };
        store
            .record_synced(folder.id, std::slice::from_ref(&entry))
            .unwrap();
        assert_eq!(
            store.synced(folder.id).unwrap(),
            std::slice::from_ref(&entry)
        );

        store
            .relocate_folder(folder.id, Path::new("/Volumes/X/Notes"), 10, None)
            .unwrap();
        let moved = store.folder(folder.id).unwrap().unwrap();
        assert_eq!(
            (moved.path.to_str(), moved.file_id, moved.bookmark),
            (Some("/Volumes/X/Notes"), 10, None)
        );

        store.remove_folder(folder.id).unwrap();
        assert!(store.folders().unwrap().is_empty());
        store.record_synced(folder.id, &[entry]).unwrap();
        assert!(
            store.synced(folder.id).unwrap().is_empty(),
            "no longer synced"
        );
    }
}
