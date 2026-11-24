//! What blob collection keeps: the blobs this device knows of, since when
//! no version it holds references each one, and the blobs each chunk table
//! lists, so that a collection downloads only the tables it never read.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{STATE_UNREADABLE, Store, array};
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

impl Store {
    /// Blobs about to be uploaded, or reused by a new version, at `at`
    /// (Unix ms). A blob no version references counts as unreferenced from
    /// then, so an upload whose version is never written goes too.
    pub(crate) fn note_blobs(&self, hashes: &[Sha256Hash], at: i64) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            // SQLite's max() is NULL when either side is: a referenced blob
            // stays referenced.
            let mut note = tx.prepare_cached(
                "INSERT INTO blobs (hash, unreferenced) VALUES (?1, ?2)
                 ON CONFLICT (hash) DO UPDATE SET
                    unreferenced = max(unreferenced, excluded.unreferenced)",
            )?;
            for hash in hashes {
                note.execute(params![hash.as_bytes(), at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that the versions this device holds reference `referenced`
    /// at `now` (Unix ms): those blobs are known and referenced, and every
    /// other known blob counts as unreferenced from `now` unless it already
    /// did. Returns each unreferenced blob, with since when.
    pub(crate) fn mark_blobs(
        &self,
        referenced: &HashSet<Sha256Hash>,
        now: i64,
    ) -> Result<Vec<(Sha256Hash, i64)>> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut known = HashMap::new();
        {
            let mut statement = tx.prepare("SELECT hash, unreferenced FROM blobs")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let since: Option<i64> = row.get(1)?;
                known.insert(Sha256Hash(array(row.get(0)?)?), since);
            }
        }
        let mut unreferenced = Vec::new();
        {
            let mut set = tx.prepare_cached(
                "INSERT INTO blobs (hash, unreferenced) VALUES (?1, ?2)
                 ON CONFLICT (hash) DO UPDATE SET unreferenced = excluded.unreferenced",
            )?;
            for hash in referenced {
                if known.get(hash) != Some(&None) {
                    set.execute(params![hash.as_bytes(), None::<i64>])?;
                }
            }
            for (hash, since) in known {
                if referenced.contains(&hash) {
                    continue;
                }
                let since = match since {
                    Some(since) => since,
                    None => {
                        set.execute(params![hash.as_bytes(), now])?;
                        now
                    }
                };
                unreferenced.push((hash, since));
            }
        }
        tx.commit()?;
        Ok(unreferenced)
    }

    /// No version referenced `hash` since `cutoff` (Unix ms), and none was
    /// noted since.
    pub(crate) fn unreferenced_since(&self, hash: &Sha256Hash, cutoff: i64) -> Result<bool> {
        Ok(self.lock().query_row(
            "SELECT EXISTS (SELECT 1 FROM blobs WHERE hash = ?1 AND unreferenced <= ?2)",
            params![hash.as_bytes(), cutoff],
            |row| row.get(0),
        )?)
    }

    /// `hash` was deleted from the servers: forgets it, unless a new version
    /// noted it after `cutoff`.
    pub(crate) fn forget_blob(&self, hash: &Sha256Hash, cutoff: i64) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.prepare_cached("DELETE FROM blobs WHERE hash = ?1 AND unreferenced <= ?2")?
            .execute(params![hash.as_bytes(), cutoff])?;
        tx.prepare_cached("DELETE FROM tables WHERE hash = ?1")?
            .execute([hash.as_bytes()])?;
        tx.commit()?;
        Ok(())
    }

    /// The blobs the chunk table `table` lists, once read.
    pub(crate) fn table_chunks(&self, table: &Sha256Hash) -> Result<Option<Vec<Sha256Hash>>> {
        let chunks: Option<Vec<u8>> = self
            .lock()
            .prepare_cached("SELECT chunks FROM tables WHERE hash = ?1")?
            .query_row([table.as_bytes()], |row| row.get(0))
            .optional()?;
        let Some(chunks) = chunks else {
            return Ok(None);
        };
        let (hashes, rest) = chunks.as_chunks::<32>();
        if !rest.is_empty() {
            return Err(Error::Malformed("corrupt local database"));
        }
        Ok(Some(hashes.iter().copied().map(Sha256Hash).collect()))
    }

    /// A table is a blob: what it lists never changes.
    pub(crate) fn set_table_chunks(&self, table: &Sha256Hash, chunks: &[Sha256Hash]) -> Result<()> {
        let bytes: Vec<u8> = chunks.iter().flat_map(|hash| *hash.as_bytes()).collect();
        self.lock()
            .prepare_cached("INSERT OR IGNORE INTO tables (hash, chunks) VALUES (?1, ?2)")?
            .execute(params![table.as_bytes(), bytes])?;
        Ok(())
    }

    /// Some wrap holds a version this device cannot read, such as a later
    /// format.
    pub(crate) fn holds_unreadable(&self) -> Result<bool> {
        Ok(self.lock().query_row(
            "SELECT EXISTS (SELECT 1 FROM wraps WHERE state = ?1)",
            [STATE_UNREADABLE],
            |row| row.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_count_as_unreferenced_from_the_last_time_they_were_used() {
        let store = Store::memory().unwrap();
        let (a, b) = (Sha256Hash([1; 32]), Sha256Hash([2; 32]));
        store.note_blobs(&[a], 10).unwrap();
        assert_eq!(store.mark_blobs(&HashSet::new(), 20).unwrap(), [(a, 10)]);

        store.note_blobs(&[a], 30).unwrap();
        assert_eq!(store.mark_blobs(&HashSet::new(), 40).unwrap(), [(a, 30)]);
        assert!(store.unreferenced_since(&a, 30).unwrap());
        assert!(!store.unreferenced_since(&a, 29).unwrap());

        let referenced = HashSet::from([a, b]);
        assert!(store.mark_blobs(&referenced, 50).unwrap().is_empty());
        store.note_blobs(&[a], 60).unwrap();
        assert!(
            !store.unreferenced_since(&a, 70).unwrap(),
            "still referenced"
        );

        let mut left = store.mark_blobs(&HashSet::from([b]), 80).unwrap();
        assert_eq!(left, [(a, 80)]);
        store.forget_blob(&a, 79).unwrap();
        left = store.mark_blobs(&HashSet::from([b]), 90).unwrap();
        assert_eq!(left, [(a, 80)], "noted after the cutoff");
        store.forget_blob(&a, 80).unwrap();
        assert!(
            store
                .mark_blobs(&HashSet::from([b]), 100)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn tables_are_read_once() {
        let store = Store::memory().unwrap();
        let table = Sha256Hash([9; 32]);
        assert_eq!(store.table_chunks(&table).unwrap(), None);
        let chunks = [Sha256Hash([1; 32]), Sha256Hash([2; 32])];
        store.set_table_chunks(&table, &chunks).unwrap();
        assert_eq!(store.table_chunks(&table).unwrap().unwrap(), chunks);
        store.forget_blob(&table, 0).unwrap();
        assert_eq!(store.table_chunks(&table).unwrap(), None);
    }
}
