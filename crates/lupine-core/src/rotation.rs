//! Key rotation (see `docs/protocol.md`): the device that rotates moves the
//! drive to a new key, then retires the old one, and every other device
//! follows once the user signs in again.
//!
//! Only [`prepare`] needs the user's key: it signs the new descriptor and
//! keeps it in the local database, so [`complete`] resumes after a crash.
//! The new key's wraps are built in a scratch database next to the local
//! one, which the move then takes in, in one transaction.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::{StreamExt, stream};
use nostr::event::{Event, EventId};
use nostr::filter::Filter;
use nostr::key::SecretKey;
use nostr::types::RelayUrl;

use crate::account::Account;
use crate::blossom::BlossomStore;
use crate::collection::references;
use crate::descriptor::{self, Descriptor, UserSigner};
use crate::drive::{DELETION_BATCH, DriveKeys};
use crate::error::{Error, Result};
use crate::ids::VersionId;
use crate::items::ItemChanges;
use crate::lists::check_servers;
use crate::relays::{Relays, Sent};
use crate::replica::{Replica, WrapState};
use crate::session::DriveSession;
use crate::store::{DescriptorRecord, DriveRecord, Move, Rotation, Store};

const WRAP_PAGE: usize = 2000;
const CLAIM_CONCURRENCY: usize = 8;
/// Syncs of the scratch database until no relay lacks a wrap.
const SPREAD_ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Publishing the new key's relays and servers.
    Lists,
    /// Asking servers to keep each blob for the new key: done, total.
    Claiming(usize, usize),
    /// Wrapping versions again: done, total.
    Wrapping(usize, usize),
    /// Sending the new wraps to every drive relay.
    Sending,
    /// Publishing the new descriptor and the old key's request to vanish.
    Publishing,
    /// Moving the local database to the new key.
    Moving,
}

#[derive(Debug, Default)]
pub struct Rotated {
    /// Versions wrapped for the new key.
    pub wrapped: usize,
    /// Blob copies that servers keep for the new key.
    pub claimed: usize,
    /// Blob copies left under the old key only, on servers that do not
    /// mirror or that refused.
    pub unclaimed: usize,
}

/// Starts a rotation, with the user's key: signs the descriptor of the new
/// key and the deletion of the older descriptors, and keeps both in the
/// local database. Fails before signing anything when a server would refuse
/// the new key. Returns the new drive secret, for the device to keep before
/// it calls [`complete`].
pub async fn prepare<S: UserSigner>(
    store: &Store,
    keys: &DriveKeys,
    account: &Account<S>,
) -> Result<SecretKey> {
    let drive = drive(store)?;
    if drive.pubkey != keys.public_key() {
        return Err(Error::Malformed("this key does not open the drive"));
    }
    if store.rotation()?.is_some() {
        return Err(Error::Rotation("a key rotation is under way"));
    }
    if store.retiring()?.is_some() {
        return Err(Error::Rotation(
            "the previous drive key is still being retired",
        ));
    }
    let kept = store.descriptor()?;
    if kept
        .as_ref()
        .is_some_and(|kept| kept.event.pubkey != account.public_key())
    {
        return Err(Error::Rotation(
            "sign in with the account the drive belongs to",
        ));
    }
    let found = account.descriptors(drive.id).await?;
    if found.iter().any(|f| f.descriptor.epoch > drive.epoch) {
        return Err(Error::Rotation(
            "another device rotated the drive key: sign in again",
        ));
    }
    let name = found
        .iter()
        .max_by_key(|f| (f.descriptor.epoch, f.event.created_at))
        .map(|f| f.descriptor.name.clone())
        .or_else(|| kept.as_ref().map(|kept| kept.name.clone()))
        .unwrap_or_else(|| "Lupine".into());
    let next = Descriptor {
        id: drive.id,
        epoch: drive.epoch + 1,
        secret: SecretKey::generate(),
        name,
    };
    check_servers(&drive.servers, &next.keys()).await?;

    let signer = &**account.signer();
    let event = next.seal(signer).await?;
    let mut older: Vec<EventId> = found.iter().map(|f| f.event.id).collect();
    older.extend(kept.map(|kept| kept.event.id));
    older.sort();
    older.dedup();
    let deletion = match older.is_empty() {
        true => None,
        false => Some(descriptor::deletion(signer, &older).await?),
    };
    store.set_rotation(Some(&Rotation {
        epoch: next.epoch,
        pubkey: next.keys().public_key(),
        descriptor: event,
        deletion,
        name: next.name.clone(),
        user_relays: account.relays().urls().to_vec(),
        committed: false,
        marker: None,
    }))?;
    Ok(next.secret)
}

/// Finishes the rotation [`prepare`] started, with the drive keys only:
/// publishes the new key's lists, asks servers to keep the blobs for it,
/// wraps every version again and sends the wraps to every drive relay.
/// Then it commits: publishes the descriptor, through `account` when the
/// user is still signed in, and the old key's request to vanish, and moves
/// the local database to the new key. Stop every sync of the drive first.
/// A run cut short resumes where it stopped; once committed, it can only
/// be finished.
pub async fn complete<S: UserSigner>(
    store: &Store,
    scratch: &Path,
    old: &DriveKeys,
    new: &DriveKeys,
    account: Option<&Account<S>>,
    progress: impl Fn(Stage),
) -> Result<Rotated> {
    let mut rotation = store
        .rotation()?
        .ok_or(Error::Rotation("no key rotation is under way"))?;
    let drive = drive(store)?;
    if drive.pubkey != old.public_key() || rotation.pubkey != new.public_key() {
        return Err(Error::Rotation("these are not the keys of the rotation"));
    }
    let moved = DriveRecord {
        pubkey: new.public_key(),
        epoch: rotation.epoch,
        ..drive.clone()
    };
    let mut rotated = Rotated::default();
    if !rotation.committed {
        progress(Stage::Lists);
        publish_lists(new, &drive, &rotation.user_relays).await?;
        (rotated.claimed, rotated.unclaimed) = claim(store, new, &drive, &progress).await;
        let scratch_store = open_scratch(scratch, &moved)?;
        rotated.wrapped = wrap_all(store, &scratch_store, old, new, &progress)?;
        progress(Stage::Sending);
        let mut session = DriveSession::open(scratch_store, copy(new)).await?;
        let spread = spread(&mut session).await;
        session.shutdown().await;
        spread?;
        rotation.committed = true;
        store.set_rotation(Some(&rotation))?;
    }

    progress(Stage::Publishing);
    publish_descriptor(&rotation, &drive, new, account).await?;
    let marker = match &rotation.marker {
        Some(marker) => marker.clone(),
        None => {
            let marker = old.vanish(vanish_targets(&drive, &rotation.user_relays))?;
            rotation.marker = Some(marker.clone());
            store.set_rotation(Some(&rotation))?;
            marker
        }
    };
    announce(&marker, old, &drive, &rotation.user_relays).await;

    progress(Stage::Moving);
    let descriptor = DescriptorRecord {
        event: rotation.descriptor.clone(),
        name: rotation.name.clone(),
        user_relays: rotation.user_relays.clone(),
    };
    rotated.wrapped += store.transplant(
        scratch,
        Move {
            old,
            new,
            drive: &moved,
            descriptor: Some(&descriptor),
            marker: &marker,
        },
    )?;
    remove_scratch(scratch)?;
    Ok(rotated)
}

/// Gives up a rotation that has not committed. Its scratch database goes,
/// and the wraps already sent go with a request to vanish from the new
/// key, when given.
pub async fn abandon(store: &Store, scratch: &Path, new: Option<&DriveKeys>) -> Result<()> {
    let Some(rotation) = store.rotation()? else {
        return Ok(());
    };
    if rotation.committed {
        return Err(Error::Rotation(
            "the new drive key is announced: the rotation can only be finished",
        ));
    }
    store.set_rotation(None)?;
    remove_scratch(scratch)?;
    if let Some(new) = new.filter(|new| new.public_key() == rotation.pubkey) {
        let drive = drive(store)?;
        let vanish = new.vanish(drive.relays.iter().cloned())?;
        if let Ok(relays) = Relays::for_drive(new, drive.relays).await {
            relays.send_all(&vanish).await;
            relays.shutdown().await;
        }
    }
    Ok(())
}

/// Moves this device's database to the key of a rotation made elsewhere,
/// which the user's key or a new recovery key gave: syncs the new key in
/// `scratch`, then moves, wrapping again the versions only this device
/// holds. `old` is the key the database is on. Stop every sync of the drive
/// first. Returns the versions wrapped again.
pub async fn adopt(
    store: &Store,
    scratch: &Path,
    old: &DriveKeys,
    new: &DriveKeys,
    moved: &DriveRecord,
    descriptor: Option<&DescriptorRecord>,
) -> Result<usize> {
    let held = drive(store)?;
    if held.id != moved.id || held.pubkey != old.public_key() || moved.pubkey != new.public_key() {
        return Err(Error::Rotation("these are not the drive's keys"));
    }
    if moved.epoch <= held.epoch {
        return Err(Error::Rotation(
            "this key is from before the drive's last key rotation",
        ));
    }
    let scratch_store = open_scratch(scratch, moved)?;
    let mut session = DriveSession::open(scratch_store, copy(new)).await?;
    let synced = session.sync().await;
    session.shutdown().await;
    drop(session);
    let synced = synced?;
    if synced.retired {
        return Err(Error::Rotation(
            "the drive key changed again: sign in again",
        ));
    }
    if let Some((_, e)) = synced.failed.into_iter().next() {
        return Err(e);
    }
    let user_relays = descriptor
        .map(|d| d.user_relays.clone())
        .unwrap_or_default();
    let marker = old.vanish(vanish_targets(&held, &user_relays))?;
    let wrapped = store.transplant(
        scratch,
        Move {
            old,
            new,
            drive: moved,
            descriptor,
            marker: &marker,
        },
    )?;
    remove_scratch(scratch)?;
    Ok(wrapped)
}

/// Deletes, with the key a rotation retired, every wrap addressed to it.
/// Sends its request to vanish first, and the deletions only to the relays
/// that then serve it, so that devices still on that key stop before they
/// apply them: a relay may honor the request without keeping it. Returns
/// whether the key is retired; what is left waits for the next call.
pub async fn retire(store: &Store, old: &DriveKeys) -> Result<bool> {
    let Some((pubkey, marker)) = store.retiring()? else {
        return Ok(true);
    };
    if pubkey != old.public_key() {
        return Err(Error::Rotation("this is not the retired key"));
    }
    let drive = drive(store)?;
    if let Some(kept) = store.descriptor()?
        && let Ok(user) = Relays::public(kept.user_relays).await
    {
        user.send_all(&marker).await;
        user.shutdown().await;
    }
    let relays = Relays::for_drive(old, drive.relays).await?;
    let sent = relays.send_all(&marker).await;
    let serving = serving(&relays, &marker).await;
    let retired = match serving.is_empty() {
        true => Err(Error::NotPublished(sent.reasons())),
        false => delete_retired(store, old, &relays, &serving).await,
    };
    relays.shutdown().await;
    retired
}

/// The relays that serve `marker` when asked.
async fn serving(relays: &Relays, marker: &Event) -> Vec<RelayUrl> {
    let filter = Filter::new()
        .id(marker.id)
        .kind(marker.kind)
        .author(marker.pubkey);
    let mut serving = Vec::new();
    for url in relays.urls() {
        if let Ok(found) = relays.events(url, &filter).await
            && found.iter().any(|event| event.id == marker.id)
        {
            serving.push(url.clone());
        }
    }
    serving
}

async fn delete_retired(
    store: &Store,
    old: &DriveKeys,
    relays: &Relays,
    marked: &[RelayUrl],
) -> Result<bool> {
    loop {
        let wraps = store.retired_wraps(DELETION_BATCH)?;
        if wraps.is_empty() {
            return store.deleted_retired(&[]);
        }
        for deletion in old.deletions(wraps.iter().copied())? {
            let sent = relays.send(&deletion, marked).await;
            if sent.accepted.is_empty() {
                return Err(Error::NotPublished(sent.reasons()));
            }
        }
        if store.deleted_retired(&wraps)? {
            return Ok(true);
        }
    }
}

fn drive(store: &Store) -> Result<DriveRecord> {
    store
        .drive()?
        .ok_or(Error::Malformed("no drive in the local database"))
}

fn copy(keys: &DriveKeys) -> DriveKeys {
    DriveKeys::from_secret(keys.keys().secret_key().clone())
}

fn vanish_targets(drive: &DriveRecord, user_relays: &[RelayUrl]) -> Vec<RelayUrl> {
    let mut targets = drive.relays.clone();
    for relay in user_relays {
        if !targets.contains(relay) {
            targets.push(relay.clone());
        }
    }
    targets
}

async fn publish_lists(
    new: &DriveKeys,
    drive: &DriveRecord,
    user_relays: &[RelayUrl],
) -> Result<()> {
    let events = [
        new.relay_list(drive.relays.iter().cloned())?,
        new.server_list(&drive.servers)?,
    ];
    let on_drive = Relays::for_drive(new, drive.relays.clone()).await?;
    let user = Relays::public(user_relays.to_vec()).await.ok();
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
    match refused {
        Some(reasons) => Err(Error::NotPublished(reasons)),
        None => Ok(()),
    }
}

/// Asks each server that mirrors (BUD-13) to keep every blob the drive
/// references for the new key, from its own copy, so no bytes leave the
/// device. Returns the copies kept, and those left under the old key only.
async fn claim(
    store: &Store,
    new: &DriveKeys,
    drive: &DriveRecord,
    progress: &impl Fn(Stage),
) -> (usize, usize) {
    let leaving = store.lists_state().map(|s| s.leaving).unwrap_or_default();
    let blobs = BlossomStore::new(&drive.servers, new.keys())
        .and_then(|blobs| blobs.with_sources(&leaving, new.keys()));
    let Ok(blobs) = blobs else {
        return (0, 0);
    };
    let Ok(referenced) = references(store, &blobs).await else {
        return (0, 0);
    };
    let hashes: Vec<_> = referenced.blobs.into_iter().collect();
    let mut mirroring = Vec::new();
    let mut unclaimed = 0;
    for server in blobs.servers() {
        if server.can_mirror().await {
            mirroring.push(server);
        } else {
            unclaimed += hashes.len();
        }
    }
    let mut jobs = Vec::with_capacity(hashes.len() * mirroring.len());
    for hash in &hashes {
        jobs.extend((0..mirroring.len()).map(|server| (server, *hash)));
    }
    let total = jobs.len();
    progress(Stage::Claiming(0, total));
    let mirroring = &mirroring;
    let mut claims = stream::iter(jobs)
        .map(|(server, hash)| async move {
            let server = mirroring[server];
            server.mirror(hash, &[server.blob_url(&hash)]).await
        })
        .buffer_unordered(CLAIM_CONCURRENCY);
    let (mut done, mut claimed) = (0, 0);
    while let Some(claim) = claims.next().await {
        done += 1;
        progress(Stage::Claiming(done, total));
        match claim {
            Ok(()) => claimed += 1,
            Err(_) => unclaimed += 1,
        }
    }
    (claimed, unclaimed)
}

/// Wraps every version this device holds again for the new key, with the
/// later formats it keeps, into the scratch database. Skips what a run cut
/// short already wrapped. Returns the wraps made.
fn wrap_all(
    store: &Store,
    scratch: &Store,
    old: &DriveKeys,
    new: &DriveKeys,
    progress: &impl Fn(Stage),
) -> Result<usize> {
    let mut replica = Replica::restore(new.public_key(), scratch.snapshot()?);
    let mut unreadable: HashSet<VersionId> = scratch
        .unreadable_events()?
        .iter()
        .filter_map(|event| new.plaintext(event))
        .map(|plaintext| VersionId::of(&plaintext))
        .collect();

    let total = store.kept_count()?;
    let (clock, _) = scratch.device_state()?;
    let (mut done, mut wrapped, mut after) = (0, 0, None);
    progress(Stage::Wrapping(0, total));
    loop {
        let page = store.kept_wraps(after, WRAP_PAGE)?;
        let Some((_, last)) = page.last() else {
            break;
        };
        after = Some(last.id);
        for (state, event) in &page {
            if let WrapState::Version(id) = state
                && replica.version(id).is_some()
            {
                continue;
            }
            let Some((id, rewrapped)) = new.rewrap(old, event) else {
                continue;
            };
            let fresh = match state {
                WrapState::Version(version) => *version == id,
                _ => unreadable.insert(id),
            };
            if fresh {
                replica.insert(new, &rewrapped);
                wrapped += 1;
            }
        }
        scratch.commit(
            &replica.take_journal(),
            clock,
            false,
            &ItemChanges::default(),
        )?;
        done += page.len();
        progress(Stage::Wrapping(done, total));
    }
    Ok(wrapped)
}

/// Syncs the scratch database until every drive relay holds its wraps.
async fn spread(session: &mut DriveSession) -> Result<()> {
    for _ in 0..SPREAD_ATTEMPTS {
        let synced = session.sync().await?;
        if synced.retired {
            return Err(Error::Rotation("the new drive key was retired"));
        }
        if let Some((_, e)) = synced.failed.into_iter().next() {
            return Err(e);
        }
        if synced.lacking == 0 {
            return Ok(());
        }
    }
    Err(Error::Rotation("the drive relays did not take every wrap"))
}

/// To the user's relays, where devices look for it, then to the drive
/// relays. The deletion of the older descriptors follows once a user relay
/// took it.
async fn publish_descriptor<S: UserSigner>(
    rotation: &Rotation,
    drive: &DriveRecord,
    new: &DriveKeys,
    account: Option<&Account<S>>,
) -> Result<()> {
    let sent = match account {
        Some(account) => send_descriptor(account.relays(), rotation).await,
        None => {
            let user = Relays::public(rotation.user_relays.clone()).await?;
            let sent = send_descriptor(&user, rotation).await;
            user.shutdown().await;
            sent
        }
    };
    if sent.accepted.is_empty() {
        return Err(Error::NotPublished(sent.reasons()));
    }
    if let Ok(on_drive) = Relays::for_drive(new, drive.relays.clone()).await {
        on_drive.send_all(&rotation.descriptor).await;
        on_drive.shutdown().await;
    }
    Ok(())
}

async fn send_descriptor(user: &Relays, rotation: &Rotation) -> Sent {
    let sent = user.send_all(&rotation.descriptor).await;
    if !sent.accepted.is_empty()
        && let Some(deletion) = &rotation.deletion
    {
        user.send_all(deletion).await;
    }
    sent
}

/// The old key's request to vanish, to the drive relays and the user's.
/// [`retire`] sends it again before any deletion.
async fn announce(marker: &Event, old: &DriveKeys, drive: &DriveRecord, user_relays: &[RelayUrl]) {
    if let Ok(relays) = Relays::for_drive(old, drive.relays.clone()).await {
        relays.send_all(marker).await;
        relays.shutdown().await;
    }
    if let Ok(relays) = Relays::public(user_relays.to_vec()).await {
        relays.send_all(marker).await;
        relays.shutdown().await;
    }
}

/// A scratch database for `drive`, kept from a run cut short when it holds
/// the same key.
fn open_scratch(path: &Path, drive: &DriveRecord) -> Result<Arc<Store>> {
    if let Ok(store) = Store::open(path) {
        match store.drive() {
            Ok(Some(held)) if held.pubkey == drive.pubkey => return Ok(Arc::new(store)),
            Ok(None) => {
                store.set_drive(drive)?;
                return Ok(Arc::new(store));
            }
            _ => {}
        }
    }
    remove_scratch(path)?;
    let store = Store::open(path)?;
    store.set_drive(drive)?;
    Ok(Arc::new(store))
}

pub fn remove_scratch(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = OsString::from(path.as_os_str());
        name.push(suffix);
        match std::fs::remove_file(PathBuf::from(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr::event::Kind;
    use nostr::key::Keys;
    use tempfile::TempDir;

    use super::*;
    use crate::account::DriveLists;
    use crate::blob::MemoryBlobStore;
    use crate::content::{ContentDescriptor, FileKey};
    use crate::descriptor::FoundDrive;
    use crate::testing::{TestRelay, blossom_server};
    use crate::writer::{ItemWriter, NewItem, Place};

    struct Device {
        store: Arc<Store>,
        scratch: PathBuf,
        _dir: TempDir,
    }

    impl Device {
        fn new(found: &FoundDrive, lists: &DriveLists, user_relays: &[RelayUrl]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("drive.sqlite");
            let store = Store::open(&path).unwrap();
            store
                .set_drive(&DriveRecord {
                    id: found.descriptor.id,
                    pubkey: found.descriptor.keys().public_key(),
                    epoch: found.descriptor.epoch,
                    relays: lists.relays.clone(),
                    servers: lists.servers.clone(),
                })
                .unwrap();
            store
                .set_descriptor(&DescriptorRecord {
                    event: found.event.clone(),
                    name: found.descriptor.name.clone(),
                    user_relays: user_relays.to_vec(),
                })
                .unwrap();
            Self {
                store: Arc::new(store),
                scratch: dir.path().join("drive.sqlite-rotation"),
                _dir: dir,
            }
        }

        async fn session(&self, keys: &DriveKeys) -> DriveSession {
            DriveSession::open(Arc::clone(&self.store), copy(keys))
                .await
                .unwrap()
        }
    }

    struct Setup {
        account: Account<Keys>,
        found: FoundDrive,
        lists: DriveLists,
        user_relays: Vec<RelayUrl>,
        _relays: Vec<TestRelay>,
    }

    impl Setup {
        async fn new() -> Self {
            Self::with(TestRelay::start().await).await
        }

        /// The second drive relay refuses `kinds`.
        async fn refusing(kinds: &'static [Kind]) -> Self {
            Self::with(TestRelay::with(|builder| builder.blacklist_kinds(kinds)).await).await
        }

        async fn with(second: TestRelay) -> Self {
            let relays = vec![TestRelay::start().await, TestRelay::start().await, second];
            let user_relays = vec![relays[0].url.clone()];
            let account = Account::connect(Arc::new(Keys::generate()), user_relays.clone())
                .await
                .unwrap();
            let drive_relays = [relays[1].url.clone(), relays[2].url.clone()];
            let servers = [blossom_server(None).await];
            let (found, lists) = account
                .create_drive("Lupine", &drive_relays, &servers)
                .await
                .unwrap();
            Self {
                account,
                found,
                lists,
                user_relays,
                _relays: relays,
            }
        }

        fn device(&self) -> Device {
            Device::new(&self.found, &self.lists, &self.user_relays)
        }
    }

    fn content(byte: u8) -> ContentDescriptor {
        ContentDescriptor {
            size: 1,
            chunk_size: 1 << 16,
            chunks: Some(vec![byte; 64]),
            table: None,
        }
    }

    fn listing(session: &DriveSession) -> Vec<String> {
        let tree = session.tree();
        let mut paths: Vec<String> = tree
            .entries()
            .map(|e| tree.path(&e.node).unwrap().join("/"))
            .collect();
        paths.sort();
        paths
    }

    async fn synced(session: &mut DriveSession) {
        let report = session.sync().await.unwrap();
        assert!(report.failed.is_empty() && !report.retired, "{report:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rotation_moves_every_device_to_the_new_key() {
        let s = Setup::new().await;
        let old = s.found.descriptor.keys();
        let (here, there) = (s.device(), s.device());
        let mut session = here.session(&old).await;
        let (photos, folder) = session.create_folder(None, "Photos").await.unwrap();
        let key = FileKey::generate();
        session
            .create_file(Some(photos), "beach.jpg", &key, content(1), None)
            .await
            .unwrap();
        let mut elsewhere = there.session(&old).await;
        synced(&mut elsewhere).await;
        let offline =
            ItemWriter::new(Arc::clone(&there.store), copy(&old), MemoryBlobStore::new()).unwrap();
        let unsent = NewItem {
            request: "unsent".into(),
            place: Place {
                parent: None,
                name: "Unsent".into(),
            },
            folder: true,
            may_exist: false,
            created: None,
            modified: None,
        };
        offline.create(unsent, None, &|_| {}).await.unwrap();

        session.shutdown().await;
        drop(session);
        let new = DriveKeys::from_secret(prepare(&here.store, &old, &s.account).await.unwrap());
        let rotated = complete(
            &here.store,
            &here.scratch,
            &old,
            &new,
            Some(&s.account),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(rotated.wrapped, 2);
        assert!(!here.scratch.exists());
        let drives = s.account.drives().await.unwrap();
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].descriptor.epoch, 1);
        assert_eq!(drives[0].descriptor.keys().public_key(), new.public_key());
        let moved = here.store.drive().unwrap().unwrap();
        assert_eq!((moved.pubkey, moved.epoch), (new.public_key(), 1));

        let mut session = here.session(&new).await;
        assert_eq!(listing(&session), ["Photos", "Photos/beach.jpg"]);
        synced(&mut session).await;
        assert!(retire(&here.store, &old).await.unwrap());
        assert!(here.store.retiring().unwrap().is_none());
        for relay in &s._relays[1..] {
            assert!(!relay.holds(folder.wrap).await, "the old key's wraps go");
        }

        let stopped = elsewhere.sync().await.unwrap();
        assert!(stopped.retired, "the old key vanished");
        assert_eq!(
            listing(&elsewhere),
            ["Photos", "Photos/beach.jpg", "Unsent"],
            "the old key's deletions never reach the tree"
        );
        assert_eq!(
            there.store.unsent().unwrap(),
            1,
            "nothing goes to the old key"
        );
        elsewhere.shutdown().await;
        drop(elsewhere);

        let current = &drives[0];
        let record = DriveRecord {
            pubkey: new.public_key(),
            epoch: current.descriptor.epoch,
            ..there.store.drive().unwrap().unwrap()
        };
        let descriptor = DescriptorRecord {
            event: current.event.clone(),
            name: current.descriptor.name.clone(),
            user_relays: s.user_relays.clone(),
        };
        let adopted = adopt(
            &there.store,
            &there.scratch,
            &old,
            &new,
            &record,
            Some(&descriptor),
        )
        .await
        .unwrap();
        assert_eq!(adopted, 1, "the folder it had not sent");
        let mut elsewhere = there.session(&new).await;
        synced(&mut elsewhere).await;
        assert_eq!(
            listing(&elsewhere),
            ["Photos", "Photos/beach.jpg", "Unsent"]
        );
        synced(&mut session).await;
        assert_eq!(listing(&session), ["Photos", "Photos/beach.jpg", "Unsent"]);
        assert!(retire(&there.store, &old).await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn old_wraps_go_only_from_relays_that_keep_the_request_to_vanish() {
        let s = Setup::with(TestRelay::forgetting(&[Kind::RequestToVanish]).await).await;
        let old = s.found.descriptor.keys();
        let here = s.device();
        let mut session = here.session(&old).await;
        let (_, folder) = session.create_folder(None, "Photos").await.unwrap();
        session.shutdown().await;
        drop(session);

        let new = DriveKeys::from_secret(prepare(&here.store, &old, &s.account).await.unwrap());
        complete(
            &here.store,
            &here.scratch,
            &old,
            &new,
            Some(&s.account),
            |_| {},
        )
        .await
        .unwrap();
        assert!(retire(&here.store, &old).await.unwrap());
        let (keeping, forgetting) = (&s._relays[1], &s._relays[2]);
        assert!(!keeping.holds(folder.wrap).await);
        assert!(
            forgetting.holds(folder.wrap).await,
            "no deletion without the request beside it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rotation_commits_only_once_every_relay_holds_the_new_wraps() {
        let s = Setup::refusing(&[Kind::GiftWrap]).await;
        let old = s.found.descriptor.keys();
        let here = s.device();
        let mut session = here.session(&old).await;
        session.create_folder(None, "Photos").await.unwrap();
        session.shutdown().await;
        drop(session);

        let new = DriveKeys::from_secret(prepare(&here.store, &old, &s.account).await.unwrap());
        let refused = complete(
            &here.store,
            &here.scratch,
            &old,
            &new,
            Some(&s.account),
            |_| {},
        )
        .await;
        assert!(matches!(refused, Err(Error::Rotation(_))), "{refused:?}");
        let rotation = here.store.rotation().unwrap().unwrap();
        assert!(!rotation.committed);
        assert_eq!(s.account.drives().await.unwrap()[0].descriptor.epoch, 0);
    }

    #[tokio::test]
    async fn a_rotation_can_be_abandoned_until_it_commits() {
        let s = Setup::new().await;
        let old = s.found.descriptor.keys();
        let here = s.device();
        let secret = prepare(&here.store, &old, &s.account).await.unwrap();
        let again = prepare(&here.store, &old, &s.account).await;
        assert!(matches!(again, Err(Error::Rotation(_))), "one at a time");
        let new = DriveKeys::from_secret(secret);
        abandon(&here.store, &here.scratch, Some(&new))
            .await
            .unwrap();
        assert!(here.store.rotation().unwrap().is_none());

        prepare(&here.store, &old, &s.account).await.unwrap();
        let mut rotation = here.store.rotation().unwrap().unwrap();
        rotation.committed = true;
        here.store.set_rotation(Some(&rotation)).unwrap();
        let refused = abandon(&here.store, &here.scratch, None).await;
        assert!(matches!(refused, Err(Error::Rotation(_))), "{refused:?}");
    }

    #[tokio::test]
    async fn writers_on_the_old_key_stop() {
        let s = Setup::new().await;
        let old = s.found.descriptor.keys();
        let here = s.device();
        let writer =
            ItemWriter::new(Arc::clone(&here.store), copy(&old), MemoryBlobStore::new()).unwrap();
        let moved = DriveKeys::generate().public_key();
        here.store
            .lock()
            .execute("UPDATE drive SET pubkey = ?1", [moved.as_bytes()])
            .unwrap();
        let folder = NewItem {
            request: "late".into(),
            place: Place {
                parent: None,
                name: "Late".into(),
            },
            folder: true,
            may_exist: false,
            created: None,
            modified: None,
        };
        let refused = writer.create(folder, None, &|_| {}).await;
        assert!(
            matches!(refused, Err(Error::DriveKeyChanged)),
            "{refused:?}"
        );
    }
}
