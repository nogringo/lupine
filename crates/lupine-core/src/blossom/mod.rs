//! Blossom client: retrieval (BUD-01), upload and mirroring (BUD-13, with
//! BUD-02 as fallback), authorization (BUD-11) and deletion (BUD-12).

mod auth;
mod server;
#[cfg(test)]
mod tests;

use std::ops::Range;

use bytes::Bytes;
use futures::future::{join_all, try_join, try_join_all};
use nostr::key::Keys;
use tokio::sync::OnceCell;

pub use server::BlossomServer;
use server::{ATTEMPTS, backoff, retrying};

use crate::blob::BlobStore;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;
use crate::task::blocking;

/// The drive's Blossom servers as one store: every blob goes to all of
/// them, and reads fall back from one server to the next.
pub struct BlossomStore {
    servers: Vec<BlossomServer>,
}

impl BlossomStore {
    /// `urls` in BUD-03 order, most trusted first. Uploads go to the first
    /// one, which the others mirror from.
    pub fn new<I, S>(urls: I, keys: &Keys) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let servers = urls
            .into_iter()
            .map(|url| BlossomServer::new(url.as_ref(), keys.clone()))
            .collect::<Result<Vec<_>>>()?;
        if servers.is_empty() {
            return Err(Error::Malformed("a drive needs a Blossom server"));
        }
        Ok(Self { servers })
    }

    pub fn servers(&self) -> &[BlossomServer] {
        &self.servers
    }

    fn all(&self) -> Vec<&BlossomServer> {
        self.servers.iter().collect()
    }
}

impl BlobStore for BlossomStore {
    /// The device uploads the blob once, to the first server. The others
    /// mirror it from there when they can, and get their own upload when
    /// they cannot.
    async fn put(&self, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        let (first, rest) = self.servers.split_first().expect("at least one server");
        let can_mirror = join_all(rest.iter().map(BlossomServer::can_mirror)).await;
        let (mirroring, direct): (Vec<_>, Vec<_>) =
            rest.iter().zip(can_mirror).partition(|(_, can)| *can);
        let sources = [first.blob_url(&hash)];

        let through_first = async {
            first.upload(hash, blob.clone()).await?;
            try_join_all(mirroring.iter().map(|(server, _)| async {
                match server.mirror(hash, &sources).await {
                    Ok(()) => Ok(()),
                    Err(_) => server.upload(hash, blob.clone()).await,
                }
            }))
            .await
        };
        let direct = try_join_all(
            direct
                .iter()
                .map(|(server, _)| server.upload(hash, blob.clone())),
        );
        try_join(through_first, direct).await?;
        Ok(())
    }

    async fn get(&self, hash: Sha256Hash) -> Result<Bytes> {
        fetch(&self.all(), hash, None).await
    }

    async fn get_range(&self, hash: Sha256Hash, range: Range<u64>) -> Result<Bytes> {
        fetch(&self.all(), hash, Some(range)).await
    }

    async fn delete(&self, hash: Sha256Hash) -> Result<()> {
        try_join_all(self.servers.iter().map(|s| s.delete(hash))).await?;
        Ok(())
    }

    /// Makes sure every server holds the blob, copying it to the servers
    /// that lost it from those that kept it. Run before publishing a version
    /// that references it, since another device may have collected it.
    ///
    /// `BlobNotFound` when no server has it any more.
    async fn replicate(&self, hash: Sha256Hash) -> Result<()> {
        let held = try_join_all(self.servers.iter().map(|s| retrying(|| s.has(hash)))).await?;
        let (holders, missing): (Vec<_>, Vec<_>) =
            self.servers.iter().zip(held).partition(|(_, held)| *held);
        if missing.is_empty() {
            return Ok(());
        }
        let holders: Vec<_> = holders.into_iter().map(|(server, _)| server).collect();
        if holders.is_empty() {
            return Err(Error::BlobNotFound(hash));
        }
        let sources: Vec<_> = holders.iter().map(|s| s.blob_url(&hash)).collect();
        let blob = OnceCell::new();
        try_join_all(missing.into_iter().map(|(server, _)| {
            let (holders, sources, blob) = (&holders, &sources, &blob);
            async move {
                if server.can_mirror().await && server.mirror(hash, sources).await.is_ok() {
                    return Ok(());
                }
                let blob = blob.get_or_try_init(|| fetch(holders, hash, None)).await?;
                server.upload(hash, blob.clone()).await
            }
        }))
        .await?;
        Ok(())
    }
}

/// Reads from the first server that serves the blob intact, so a server
/// that lost or corrupted it does not fail the read. Whole blobs are checked
/// against their hash here; ranges are segments that authenticate themselves.
async fn fetch(
    servers: &[&BlossomServer],
    hash: Sha256Hash,
    range: Option<Range<u64>>,
) -> Result<Bytes> {
    let mut error = Error::BlobNotFound(hash);
    for attempt in 1..=ATTEMPTS {
        let mut transient = false;
        for server in servers {
            let result = match &range {
                Some(range) => server.get_range(hash, range.clone()).await,
                None => match server.get(hash).await {
                    Ok(blob) => verify(hash, blob).await,
                    Err(e) => Err(e),
                },
            };
            match result {
                Ok(blob) => return Ok(blob),
                Err(Error::BlobNotFound(_)) => {}
                Err(e) => {
                    transient |= e.is_transient();
                    error = e;
                }
            }
        }
        if !transient || attempt == ATTEMPTS {
            break;
        }
        tokio::time::sleep(backoff(attempt)).await;
    }
    Err(error)
}

async fn verify(hash: Sha256Hash, blob: Bytes) -> Result<Bytes> {
    blocking(move || {
        if Sha256Hash::digest(&blob) == hash {
            Ok(blob)
        } else {
            Err(Error::BlobHashMismatch(hash))
        }
    })
    .await
}
