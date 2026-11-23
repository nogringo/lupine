//! Content-addressed blob storage. `blossom::BlossomStore` implements it
//! over HTTP; the memory and directory stores serve tests and offline
//! development.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

pub trait BlobStore: Send + Sync {
    /// `hash` is the SHA-256 of `blob`, computed by the caller.
    fn put(&self, hash: Sha256Hash, blob: Bytes) -> impl Future<Output = Result<()>> + Send;

    /// Implementations need not check the hash: readers always do.
    fn get(&self, hash: Sha256Hash) -> impl Future<Output = Result<Bytes>> + Send;

    /// Part of a blob. The default fetches it whole, which is also the
    /// fallback for Blossom servers that ignore `Range`.
    fn get_range(
        &self,
        hash: Sha256Hash,
        range: Range<u64>,
    ) -> impl Future<Output = Result<Bytes>> + Send {
        async move { slice(self.get(hash).await?, range) }
    }

    /// Succeeds when the blob is already absent.
    fn delete(&self, hash: Sha256Hash) -> impl Future<Output = Result<()>> + Send;

    /// Makes sure the store still holds a blob that a new version reuses:
    /// another device may have collected it. `BlobNotFound` when it is gone.
    fn replicate(&self, hash: Sha256Hash) -> impl Future<Output = Result<()>> + Send {
        async move { self.get_range(hash, 0..0).await.map(drop) }
    }
}

impl<S: BlobStore> BlobStore for std::sync::Arc<S> {
    fn put(&self, hash: Sha256Hash, blob: Bytes) -> impl Future<Output = Result<()>> + Send {
        (**self).put(hash, blob)
    }

    fn get(&self, hash: Sha256Hash) -> impl Future<Output = Result<Bytes>> + Send {
        (**self).get(hash)
    }

    fn get_range(
        &self,
        hash: Sha256Hash,
        range: Range<u64>,
    ) -> impl Future<Output = Result<Bytes>> + Send {
        (**self).get_range(hash, range)
    }

    fn delete(&self, hash: Sha256Hash) -> impl Future<Output = Result<()>> + Send {
        (**self).delete(hash)
    }

    fn replicate(&self, hash: Sha256Hash) -> impl Future<Output = Result<()>> + Send {
        (**self).replicate(hash)
    }
}

pub(crate) fn slice(blob: Bytes, range: Range<u64>) -> Result<Bytes> {
    if range.start > range.end || range.end > blob.len() as u64 {
        return Err(Error::Malformed("range outside the blob"));
    }
    Ok(blob.slice(range.start as usize..range.end as usize))
}

#[derive(Default)]
pub struct MemoryBlobStore {
    blobs: Mutex<HashMap<Sha256Hash, Bytes>>,
    puts: AtomicUsize,
    bytes_served: AtomicUsize,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn put_count(&self) -> usize {
        self.puts.load(Ordering::Relaxed)
    }

    pub fn bytes_served(&self) -> usize {
        self.bytes_served.load(Ordering::Relaxed)
    }

    pub fn blob_lens(&self) -> Vec<usize> {
        self.lock().values().map(Bytes::len).collect()
    }

    /// Replaces a stored blob without updating its address, to simulate a
    /// misbehaving server.
    pub fn overwrite(&self, hash: Sha256Hash, blob: Bytes) {
        self.lock().insert(hash, blob);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Sha256Hash, Bytes>> {
        self.blobs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn serve(&self, data: Bytes) -> Bytes {
        self.bytes_served.fetch_add(data.len(), Ordering::Relaxed);
        data
    }
}

impl BlobStore for MemoryBlobStore {
    async fn put(&self, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        self.puts.fetch_add(1, Ordering::Relaxed);
        self.lock().insert(hash, blob);
        Ok(())
    }

    async fn get(&self, hash: Sha256Hash) -> Result<Bytes> {
        let blob = self.lock().get(&hash).cloned();
        blob.map(|b| self.serve(b)).ok_or(Error::BlobNotFound(hash))
    }

    async fn get_range(&self, hash: Sha256Hash, range: Range<u64>) -> Result<Bytes> {
        let blob = self.lock().get(&hash).cloned();
        let blob = blob.ok_or(Error::BlobNotFound(hash))?;
        slice(blob, range).map(|part| self.serve(part))
    }

    async fn delete(&self, hash: Sha256Hash) -> Result<()> {
        self.lock().remove(&hash);
        Ok(())
    }
}

/// One file per blob, named by its hash.
pub struct DirBlobStore {
    root: PathBuf,
}

impl DirBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self, hash: &Sha256Hash) -> PathBuf {
        self.root.join(hash.to_hex())
    }
}

fn not_found(hash: Sha256Hash) -> impl FnOnce(io::Error) -> Error {
    move |e| match e.kind() {
        io::ErrorKind::NotFound => Error::BlobNotFound(hash),
        _ => e.into(),
    }
}

impl BlobStore for DirBlobStore {
    async fn put(&self, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        let path = self.path(&hash);
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        let temp = self.root.join(format!(
            ".{}.{}.tmp",
            hash,
            hex::encode(random_bytes::<8>())
        ));
        tokio::fs::write(&temp, &blob).await?;
        tokio::fs::rename(&temp, &path).await?;
        Ok(())
    }

    async fn get(&self, hash: Sha256Hash) -> Result<Bytes> {
        let data = tokio::fs::read(self.path(&hash))
            .await
            .map_err(not_found(hash))?;
        Ok(data.into())
    }

    async fn get_range(&self, hash: Sha256Hash, range: Range<u64>) -> Result<Bytes> {
        if range.start > range.end {
            return Err(Error::Malformed("range outside the blob"));
        }
        let mut file = tokio::fs::File::open(self.path(&hash))
            .await
            .map_err(not_found(hash))?;
        file.seek(io::SeekFrom::Start(range.start)).await?;
        let mut data = vec![0; (range.end - range.start) as usize];
        file.read_exact(&mut data)
            .await
            .map_err(|_| Error::Malformed("range outside the blob"))?;
        Ok(data.into())
    }

    async fn delete(&self, hash: Sha256Hash) -> Result<()> {
        match tokio::fs::remove_file(self.path(&hash)).await {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}
