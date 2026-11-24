//! The drive's relays and servers as this device last applied them, and
//! the move to a new server list: the servers the drive leaves stay
//! readable until every blob reached the others.

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use nostr::types::RelayUrl;

use super::Store;
use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::node::{FileMeta, NodeVersion};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListsState {
    /// `created_at` of the kinds 10050 and 10063 applied, 0 before any.
    pub relays_at: u64,
    pub servers_at: u64,
    /// Servers the drive left, still read from until the fill completes.
    pub leaving: Vec<String>,
    /// Nonzero while a fill is due. A new server list changes it.
    pub fill: i64,
}

impl Store {
    pub fn lists_state(&self) -> Result<ListsState> {
        let row = self
            .lock()
            .query_row(
                "SELECT relays_at, servers_at, leaving, fill FROM drive",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((relays_at, servers_at, leaving, fill)) = row else {
            return Ok(ListsState::default());
        };
        Ok(ListsState {
            relays_at: relays_at.max(0) as u64,
            servers_at: servers_at.max(0) as u64,
            leaving: serde_json::from_str(&leaving)?,
            fill,
        })
    }

    /// Applies the relay list dated `at`.
    pub fn set_relays(&self, relays: &[RelayUrl], at: u64) -> Result<()> {
        if relays.is_empty() {
            return Err(Error::Malformed("a drive needs a relay"));
        }
        let updated = self.lock().execute(
            "UPDATE drive SET relays = ?1, relays_at = ?2",
            params![serde_json::to_string(relays)?, at as i64],
        )?;
        no_drive(updated)
    }

    /// Moves the drive to the server list dated `at`. The servers it leaves
    /// stay readable, and a fill is due until every blob reached the list.
    pub fn set_servers(&self, servers: &[String], at: u64) -> Result<()> {
        if servers.is_empty() {
            return Err(Error::Malformed("a drive needs a Blossom server"));
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(String, String)> = tx
            .query_row("SELECT servers, leaving FROM drive", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let Some((current, leaving)) = row else {
            return Err(Error::Malformed("no drive in the local database"));
        };
        let current: Vec<String> = serde_json::from_str(&current)?;
        if current == servers {
            tx.execute("UPDATE drive SET servers_at = ?1", [at as i64])?;
        } else {
            let leaving: Vec<String> = serde_json::from_str(&leaving)?;
            let mut left = Vec::new();
            for server in leaving.into_iter().chain(current) {
                if !servers.contains(&server) && !left.contains(&server) {
                    left.push(server);
                }
            }
            let fill = i64::from_be_bytes(random_bytes()) | 1;
            tx.execute(
                "UPDATE drive SET servers = ?1, servers_at = ?2, leaving = ?3, fill = ?4",
                params![
                    serde_json::to_string(servers)?,
                    at as i64,
                    serde_json::to_string(&left)?,
                    fill
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The fill `fill` copied every blob: the servers left are dropped,
    /// unless the list changed since. Returns whether they were.
    pub fn filled(&self, fill: i64) -> Result<bool> {
        let updated = self.lock().execute(
            "UPDATE drive SET leaving = '[]', fill = 0 WHERE fill = ?1 AND fill != 0",
            [fill],
        )?;
        Ok(updated > 0)
    }

    /// The content of every file version this device holds.
    pub fn file_contents(&self) -> Result<Vec<FileMeta>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT json FROM versions")?;
        let mut rows = statement.query([])?;
        let mut files = Vec::new();
        while let Some(row) = rows.next()? {
            let version: NodeVersion = serde_json::from_str(&row.get::<_, String>(0)?)?;
            files.extend(version.file);
        }
        Ok(files)
    }
}

fn no_drive(updated: usize) -> Result<()> {
    if updated == 0 {
        return Err(Error::Malformed("no drive in the local database"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;
    use crate::ids::DriveId;
    use crate::store::DriveRecord;

    fn servers(urls: &[&str]) -> Vec<String> {
        urls.iter().map(|url| format!("https://{url}")).collect()
    }

    #[test]
    fn servers_the_drive_leaves_stay_until_filled() {
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: DriveId([1; 16]),
                pubkey: Keys::generate().public_key(),
                epoch: 0,
                relays: vec![RelayUrl::parse("wss://relay.example.com").unwrap()],
                servers: servers(&["a", "b"]),
            })
            .unwrap();
        assert_eq!(store.lists_state().unwrap(), ListsState::default());

        store.set_servers(&servers(&["a", "b"]), 5).unwrap();
        let state = store.lists_state().unwrap();
        assert_eq!((state.servers_at, state.fill), (5, 0), "nothing to fill");

        store.set_servers(&servers(&["c", "a"]), 6).unwrap();
        let first = store.lists_state().unwrap();
        assert_eq!(first.leaving, servers(&["b"]));
        assert_ne!(first.fill, 0);
        store.set_servers(&servers(&["c"]), 7).unwrap();
        let second = store.lists_state().unwrap();
        assert_eq!(second.leaving, servers(&["b", "a"]));
        assert_eq!(store.drive().unwrap().unwrap().servers, servers(&["c"]));

        assert!(!store.filled(first.fill).unwrap(), "the list changed since");
        assert!(store.filled(second.fill).unwrap());
        let done = store.lists_state().unwrap();
        assert_eq!((done.leaving.len(), done.fill, done.servers_at), (0, 0, 7));

        store.set_servers(&servers(&["a"]), 8).unwrap();
        assert_eq!(store.lists_state().unwrap().leaving, servers(&["c"]));
    }
}
