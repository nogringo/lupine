//! Creates a drive holding a copy of a local folder and prints its recovery
//! key, to try a client's read path before it can write. Publishes no
//! descriptor, since no user key is involved.
//!
//! cargo run --release -p lupine-core --example seed -- \
//!     --relay <wss://...> --server <https://...> <folder>
//!
//! `--relay` and `--server` repeat. Servers must store arbitrary blobs:
//! media hosts refuse encrypted chunks.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use lupine_core::blossom::BlossomStore;
use lupine_core::content::{self, FileKey, TransferOptions};
use lupine_core::descriptor::Descriptor;
use lupine_core::ids::NodeId;
use lupine_core::relays::Relays;
use lupine_core::session::{DriveSession, Published};
use lupine_core::store::{DriveRecord, Store};
use nostr::types::RelayUrl;

type Failure = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let (relays, servers, folder) = arguments()?;
    let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Lupine");
    let descriptor = Descriptor::generate(name);
    let keys = descriptor.keys();

    let announce = Relays::for_drive(&keys, relays.clone()).await?;
    for list in [
        keys.relay_list(relays.clone())?,
        keys.server_list(&servers)?,
    ] {
        let sent = announce.send_all(&list).await;
        if sent.accepted.is_empty() {
            return Err(format!("no relay took the drive's lists: {}", sent.reasons()).into());
        }
    }
    announce.shutdown().await;

    let store = Arc::new(Store::memory()?);
    store.set_drive(&DriveRecord {
        id: descriptor.id,
        pubkey: keys.public_key(),
        epoch: descriptor.epoch,
        relays: relays.clone(),
        servers: servers.clone(),
    })?;
    let blobs = BlossomStore::new(&servers, keys.keys())?;
    let mut session = DriveSession::open(store, descriptor.keys()).await?;

    let mut folders = vec![(folder, None::<NodeId>)];
    while let Some((dir, parent)) = folders.pop() {
        let mut entries = std::fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                eprintln!("skipped, not UTF-8: {}", path.display());
                continue;
            };
            if name == ".DS_Store" {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                let (node, published) = session.create_folder(parent, &name).await?;
                accepted(&published)?;
                folders.push((path, Some(node)));
            } else if kind.is_file() {
                let modified = entry
                    .metadata()?
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .ok();
                let key = FileKey::generate();
                let source = Arc::new(std::fs::File::open(&path)?);
                let options = TransferOptions::default();
                let uploaded =
                    content::upload(&blobs, source, &key, None, options, &|_| {}).await?;
                let described = uploaded.describe(&blobs, &key).await?;
                let (_, published) = session
                    .create_file(parent, &name, &key, described, modified)
                    .await?;
                accepted(&published)?;
                eprintln!("{}", path.display());
            }
        }
    }
    session.shutdown().await;
    println!("{}", descriptor.recovery_key(&relays).encode());
    Ok(())
}

fn accepted(published: &Published) -> Result<(), Failure> {
    if published.sent.accepted.is_empty() {
        return Err(format!("no relay took a version: {}", published.sent.reasons()).into());
    }
    Ok(())
}

fn arguments() -> Result<(Vec<RelayUrl>, Vec<String>, PathBuf), Failure> {
    let usage = "usage: seed --relay <url> --server <url> <folder>";
    let (mut relays, mut servers, mut folder) = (Vec::new(), Vec::new(), None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--relay" => relays.push(RelayUrl::parse(&args.next().ok_or(usage)?)?),
            "--server" => servers.push(args.next().ok_or(usage)?),
            _ => folder = Some(PathBuf::from(arg)),
        }
    }
    match folder {
        Some(folder) if !relays.is_empty() && !servers.is_empty() => Ok((relays, servers, folder)),
        _ => Err(usage.into()),
    }
}
