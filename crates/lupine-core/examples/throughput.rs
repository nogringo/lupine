//! Encrypts a file through the upload pipeline into a store that discards
//! blobs, to measure CPU throughput and check that memory stays bounded.
//!
//! cargo run --release -p lupine-core --example throughput -- <file> [concurrency]

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use lupine_core::blob::BlobStore;
use lupine_core::content::{FileKey, TransferOptions, upload};
use lupine_core::ids::Sha256Hash;
use lupine_core::{Error, Result};

struct DiscardStore;

impl BlobStore for DiscardStore {
    async fn put(&self, _: Sha256Hash, _: Bytes) -> Result<()> {
        Ok(())
    }

    async fn get(&self, hash: Sha256Hash) -> Result<Bytes> {
        Err(Error::BlobNotFound(hash))
    }

    async fn delete(&self, _: Sha256Hash) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: throughput <file> [concurrency]");
    let concurrency = args.next().map_or(4, |c| c.parse().expect("concurrency"));
    let source = Arc::new(std::fs::File::open(path)?);
    let options = TransferOptions {
        concurrency,
        ..TransferOptions::default()
    };

    let started = Instant::now();
    let content = upload(
        &DiscardStore,
        source,
        &FileKey::generate(),
        None,
        options,
        &|_| {},
    )
    .await?;
    let seconds = started.elapsed().as_secs_f64();
    let size = content.layout().size() as f64;
    println!(
        "{} chunks, {:.1} GiB in {seconds:.1} s: {:.0} MiB/s",
        content.chunks().len(),
        size / f64::from(1 << 30),
        size / f64::from(1 << 20) / seconds
    );
    Ok(())
}
