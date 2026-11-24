//! Changing the drive's relays (kind 10050) and Blossom servers (kind
//! 10063), on this device or on another, and filling the servers a drive
//! moves to with every blob it references.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use futures::future::try_join_all;
use futures::{StreamExt, stream};
use nostr::event::Kind;
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::types::RelayUrl;

use crate::blob::BlobStore;
use crate::blossom::{BlossomServer, BlossomStore};
use crate::content::{ChunkLayout, DEFAULT_CHUNK_SIZE};
use crate::drive::{DriveKeys, read_relay_list, read_server_list};
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;
use crate::relays::Relays;
use crate::store::Store;

const FILL_CONCURRENCY: usize = 8;

/// The latest kinds 10050 and 10063 of a drive, with their `created_at`.
#[derive(Debug, Default)]
pub struct Latest {
    pub relays: Option<(Vec<RelayUrl>, u64)>,
    pub servers: Option<(Vec<String>, u64)>,
}

impl Latest {
    pub async fn find(relays: &Relays, drive: PublicKey) -> Result<Self> {
        let filter = Filter::new()
            .kinds([Kind::InboxRelays, Kind::BlossomServerList])
            .author(drive);
        let events = relays.query(&filter).await?;
        let latest = |kind: Kind| {
            events
                .iter()
                .filter(|e| e.kind == kind && e.pubkey == drive && e.verify().is_ok())
                .max_by_key(|e| (e.created_at, Reverse(e.id)))
        };
        Ok(Self {
            relays: latest(Kind::InboxRelays)
                .map(|event| (read_relay_list(event), event.created_at.as_secs())),
            servers: latest(Kind::BlossomServerList)
                .map(|event| (read_server_list(event), event.created_at.as_secs())),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub relays: bool,
    pub servers: bool,
}

impl Changed {
    pub fn any(self) -> bool {
        self.relays || self.servers
    }
}

/// Applies the lists newer than those this device applied, as another
/// device published them. Empty lists are ignored.
pub fn apply(store: &Store, latest: Latest) -> Result<Changed> {
    let (Some(drive), state) = (store.drive()?, store.lists_state()?) else {
        return Ok(Changed::default());
    };
    let mut changed = Changed::default();
    if let Some((relays, at)) = latest.relays
        && at > state.relays_at
        && !relays.is_empty()
    {
        changed.relays = relays != drive.relays;
        store.set_relays(&relays, at)?;
    }
    if let Some((servers, at)) = latest.servers
        && at > state.servers_at
        && !servers.is_empty()
    {
        changed.servers = servers != drive.servers;
        store.set_servers(&servers, at)?;
    }
    Ok(changed)
}

/// Publishes new lists to the drive relays, old and new, and to the user's
/// relays, then applies them on this device. Fails before publishing when
/// an added server would refuse encrypted chunks, and without applying
/// anything when no drive relay takes the lists.
pub async fn change(
    store: &Store,
    keys: &DriveKeys,
    relays: Vec<RelayUrl>,
    servers: Vec<String>,
) -> Result<Changed> {
    let drive = store
        .drive()?
        .ok_or(Error::Malformed("no drive in the local database"))?;
    let origins = |servers: &[String]| {
        servers
            .iter()
            .map(|url| {
                Ok(BlossomServer::new(url, keys.keys().clone())?
                    .url()
                    .to_owned())
            })
            .collect::<Result<Vec<_>>>()
    };
    let current = origins(&drive.servers)?;
    let relays = unique(relays);
    let servers = unique(origins(&servers)?);
    if relays.is_empty() || servers.is_empty() {
        return Err(Error::Malformed(
            "a drive needs a relay and a Blossom server",
        ));
    }
    let changed = Changed {
        relays: relays != drive.relays,
        servers: servers != current,
    };
    if !changed.any() {
        return Ok(changed);
    }
    let added: Vec<String> = servers
        .iter()
        .filter(|server| !current.contains(server))
        .cloned()
        .collect();
    if !added.is_empty() {
        check_servers(&added, keys).await?;
    }

    let mut events = Vec::new();
    if changed.relays {
        events.push(keys.relay_list(relays.iter().cloned())?);
    }
    if changed.servers {
        events.push(keys.server_list(&servers)?);
    }
    let on_drive = Relays::for_drive(keys, unique([drive.relays, relays.clone()].concat())).await?;
    let user = match store.descriptor()? {
        Some(kept) => Relays::public(kept.user_relays).await.ok(),
        None => None,
    };
    let mut refused = None;
    for event in &events {
        let sent = on_drive.send_all(event).await;
        if let Some(user) = &user {
            user.send_all(event).await;
        }
        if sent.accepted.is_empty() {
            refused = Some(sent.reasons());
            break;
        }
    }
    on_drive.shutdown().await;
    if let Some(user) = user {
        user.shutdown().await;
    }
    if let Some(reasons) = refused {
        return Err(Error::NotPublished(reasons));
    }
    for event in &events {
        let at = event.created_at.as_secs();
        match event.kind {
            Kind::InboxRelays => store.set_relays(&relays, at)?,
            _ => store.set_servers(&servers, at)?,
        }
    }
    Ok(changed)
}

/// Fails with the first server that would refuse an encrypted chunk.
pub(crate) async fn check_servers(servers: &[String], keys: &DriveKeys) -> Result<()> {
    let blobs = BlossomStore::new(servers, keys.keys())?;
    let chunk = DEFAULT_CHUNK_SIZE;
    let len = ChunkLayout::new(chunk.into(), chunk)?.blob_len(0) as u64;
    try_join_all(blobs.servers().iter().map(|s| s.check_upload(len))).await?;
    Ok(())
}

fn unique<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut unique = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

#[derive(Debug, Default)]
pub struct Filled {
    /// Blobs that the versions this device holds reference.
    pub blobs: usize,
    /// On no server at all: there is nothing to copy.
    pub lost: Vec<Sha256Hash>,
    /// Left for the next fill, which keeps reading from the servers left.
    pub failed: Vec<(Sha256Hash, Error)>,
}

/// Copies every blob that the versions this device holds reference to
/// each drive server that lacks it, from those that hold it, the servers
/// the drive leaves included. Once no blob is left behind, the servers
/// left are dropped. Does nothing unless a fill is due. `progress` gets the
/// blobs done and their total.
pub async fn fill(
    store: &Store,
    keys: &DriveKeys,
    progress: impl Fn(usize, usize),
) -> Result<Filled> {
    let state = store.lists_state()?;
    let Some(drive) = store.drive()? else {
        return Ok(Filled::default());
    };
    if state.fill == 0 {
        return Ok(Filled::default());
    }
    let blobs = BlossomStore::new(&drive.servers, keys.keys())?
        .with_sources(&state.leaving, keys.keys())?;

    let mut hashes = BTreeSet::new();
    let mut tables = BTreeSet::new();
    let mut failed = BTreeMap::new();
    for file in store.file_contents()? {
        let content = &file.content;
        if let Some(table) = content.table {
            if !tables.insert(table) {
                continue;
            }
            hashes.insert(table);
        }
        match content.resolve(&blobs, &file.file_key()).await {
            Ok(resolved) => hashes.extend(resolved.chunks().iter().map(|chunk| chunk.blob)),
            Err(e) => {
                if let Some(table) = content.table {
                    failed.insert(table, e);
                }
            }
        }
    }

    let total = hashes.len();
    let mut done = 0;
    let mut lost = Vec::new();
    progress(done, total);
    let blobs = &blobs;
    let mut copies = stream::iter(hashes)
        .map(|hash| async move { (hash, blobs.replicate(hash).await) })
        .buffer_unordered(FILL_CONCURRENCY);
    while let Some((hash, copied)) = copies.next().await {
        done += 1;
        progress(done, total);
        match copied {
            Ok(()) => {}
            Err(Error::BlobNotFound(_)) => lost.push(hash),
            Err(e) => {
                failed.insert(hash, e);
            }
        }
    }
    // A table on no server at all is lost too.
    failed.retain(|hash, _| !lost.contains(hash));
    if failed.is_empty() {
        store.filled(state.fill)?;
    }
    Ok(Filled {
        blobs: total,
        lost,
        failed: failed.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::content::{self, FileKey, INLINE_CHUNK_LIMIT, TransferOptions};
    use crate::ids::DriveId;
    use crate::io::MemoryFile;
    use crate::session::DriveSession;
    use crate::store::DriveRecord;
    use crate::testing::{TestBlossom, TestRelay, blossom_server};

    fn store(drive: &DriveKeys, relays: &[&TestRelay], servers: &[&str]) -> Arc<Store> {
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: DriveId([3; 16]),
                pubkey: drive.public_key(),
                epoch: 0,
                relays: relays.iter().map(|r| r.url.clone()).collect(),
                servers: servers.iter().map(|s| s.to_string()).collect(),
            })
            .unwrap();
        Arc::new(store)
    }

    fn keys(drive: &DriveKeys) -> DriveKeys {
        DriveKeys::from_secret(drive.keys().secret_key().clone())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn moving_servers_copies_every_blob_then_drops_the_old_ones() {
        let relay = TestRelay::start().await;
        let (old, new) = (TestBlossom::start().await, TestBlossom::start().await);
        let drive = DriveKeys::generate();
        let store = store(&drive, &[&relay], &[&old.url]);
        let mut session = DriveSession::open(Arc::clone(&store), keys(&drive))
            .await
            .unwrap();
        let blobs = BlossomStore::new([&old.url], drive.keys()).unwrap();
        let options = TransferOptions {
            chunk_size: 1 << 16,
            concurrency: 4,
        };
        let large = (INLINE_CHUNK_LIMIT + 1) << 16;
        for (name, size) in [("small", 1000), ("large", large)] {
            let key = FileKey::generate();
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let source = Arc::new(MemoryFile::new(data));
            let uploaded = content::upload(&blobs, source, &key, None, options, &|_| {})
                .await
                .unwrap();
            let described = uploaded.describe(&blobs, &key).await.unwrap();
            session
                .create_file(None, name, &key, described, None)
                .await
                .unwrap();
        }
        let stored = old.len();
        assert_eq!(
            stored,
            1 + INLINE_CHUNK_LIMIT + 1 + 1,
            "with the large file's table"
        );

        let changed = change(
            &store,
            &drive,
            vec![relay.url.clone()],
            vec![new.url.clone()],
        )
        .await
        .unwrap();
        assert_eq!(
            changed,
            Changed {
                relays: false,
                servers: true
            }
        );
        let leaving = store.lists_state().unwrap().leaving;
        assert_eq!(leaving, std::slice::from_ref(&old.url));

        let filled = fill(&store, &drive, |_, _| {}).await.unwrap();
        assert_eq!(filled.blobs, stored);
        assert!(filled.failed.is_empty() && filled.lost.is_empty());
        assert_eq!(new.len(), stored);
        assert!(store.lists_state().unwrap().leaving.is_empty());
        assert_eq!(fill(&store, &drive, |_, _| {}).await.unwrap().blobs, 0);
    }

    #[tokio::test]
    async fn devices_follow_lists_changed_elsewhere() {
        let (a, b) = (TestRelay::start().await, TestRelay::start().await);
        let (first, second) = (blossom_server(None).await, blossom_server(None).await);
        let drive = DriveKeys::generate();
        let here = store(&drive, &[&a], &[&first]);
        let elsewhere = store(&drive, &[&a], &[&first]);
        let mut session = DriveSession::open(Arc::clone(&elsewhere), keys(&drive))
            .await
            .unwrap();
        assert!(!session.sync().await.unwrap().lists.any());

        let relays = vec![a.url.clone(), b.url.clone()];
        let servers = vec![first.clone(), second.clone()];
        change(&here, &drive, relays.clone(), servers.clone())
            .await
            .unwrap();
        let synced = session.sync().await.unwrap();
        assert_eq!(
            synced.lists,
            Changed {
                relays: true,
                servers: true
            }
        );
        let applied = elsewhere.drive().unwrap().unwrap();
        assert_eq!((applied.relays, applied.servers), (relays, servers));
        let state = elsewhere.lists_state().unwrap();
        assert!(state.leaving.is_empty());
        assert_ne!(state.fill, 0, "the added server gets every blob");
        assert!(!session.sync().await.unwrap().lists.any());
    }

    #[tokio::test]
    async fn lists_no_relay_takes_are_not_applied() {
        let refusing = [Kind::InboxRelays, Kind::BlossomServerList];
        let relay = TestRelay::with(|builder| builder.blacklist_kinds(&refusing)).await;
        let (server, added) = (blossom_server(None).await, blossom_server(None).await);
        let drive = DriveKeys::generate();
        let store = store(&drive, &[&relay], &[&server]);
        let servers = vec![server.clone(), added];
        let refused = change(&store, &drive, vec![relay.url.clone()], servers).await;
        assert!(
            matches!(refused, Err(Error::NotPublished(_))),
            "{refused:?}"
        );
        assert_eq!(store.drive().unwrap().unwrap().servers, [server]);
        assert_eq!(store.lists_state().unwrap().fill, 0);
    }
}
