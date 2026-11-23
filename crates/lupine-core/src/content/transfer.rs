use std::io;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use futures::{StreamExt, TryStreamExt, stream};

use super::chunk::{ChunkBuffer, ChunkRef, FileKey, open_segments};
use super::layout::{ChunkLayout, DEFAULT_CHUNK_SIZE, SEGMENT_SIZE};
use super::table::FileContent;
use crate::blob::BlobStore;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;
use crate::io::{ReadAt, WriteAt};
use crate::task::blocking;

#[derive(Clone, Copy, Debug)]
pub struct TransferOptions {
    pub chunk_size: u32,
    /// Chunks in flight. Memory use is about `concurrency * chunk_size` on
    /// upload, and at most that on download.
    pub concurrency: usize,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            concurrency: 4,
        }
    }
}

/// Receives the cumulative number of plaintext bytes transferred.
pub type Progress<'a> = &'a (dyn Fn(u64) + Send + Sync);

enum Prepared {
    Unchanged(ChunkRef),
    Sealed(ChunkRef, Bytes),
}

/// Encrypts `source` chunk by chunk and stores the blobs, in bounded memory.
///
/// `previous` is the current content of the same file. Chunks whose
/// plaintext did not change keep their blob and are not uploaded again.
pub async fn upload<S, R>(
    store: &S,
    source: Arc<R>,
    key: &FileKey,
    previous: Option<&FileContent>,
    options: TransferOptions,
    progress: Progress<'_>,
) -> Result<FileContent>
where
    S: BlobStore,
    R: ReadAt + 'static,
{
    let size = source.size()?;
    let layout = ChunkLayout::new(size, options.chunk_size)?;
    let previous = previous.filter(|p| p.layout().chunk_size() == options.chunk_size);
    let done = &AtomicU64::new(0);

    let chunks = stream::iter(0..layout.chunk_count())
        .map(|index| {
            let source = Arc::clone(&source);
            let key = key.clone();
            let unchanged = previous.and_then(|p| p.chunks().get(index as usize).copied());
            async move {
                let prepared =
                    blocking(move || prepare_chunk(&*source, &key, layout, index, unchanged))
                        .await?;
                let chunk = match prepared {
                    Prepared::Unchanged(chunk) => chunk,
                    Prepared::Sealed(chunk, blob) => {
                        store.put(chunk.blob, blob).await?;
                        chunk
                    }
                };
                let len = layout.chunk_len(index) as u64;
                progress(done.fetch_add(len, Ordering::Relaxed) + len);
                Ok::<_, Error>(chunk)
            }
        })
        .buffered(options.concurrency.max(1))
        .try_collect()
        .await?;

    if source.size()? != size {
        return Err(Error::SourceChanged);
    }
    FileContent::new(layout, chunks)
}

fn prepare_chunk<R: ReadAt + ?Sized>(
    source: &R,
    key: &FileKey,
    layout: ChunkLayout,
    index: u64,
    unchanged: Option<ChunkRef>,
) -> Result<Prepared> {
    let len = layout.chunk_len(index);
    let start = layout.chunk_range(index).start;
    let mut buffer = ChunkBuffer::new(layout.padded_len(index));
    buffer
        .fill(len, |slot, offset| {
            source.read_exact_at(slot, start + offset)
        })
        .map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof => Error::SourceChanged,
            _ => e.into(),
        })?;
    let mac = key.chunk_mac(index, buffer.plaintext(len));
    if let Some(chunk) = unchanged.filter(|c| c.mac == mac) {
        return Ok(Prepared::Unchanged(chunk));
    }
    let (blob, data) = buffer.seal(key, index, &mac);
    Ok(Prepared::Sealed(ChunkRef { blob, mac }, data))
}

/// Fetches, verifies and decrypts the segments covering `range`, and writes
/// them at their offsets in `sink`. A chunk needed whole is fetched whole
/// and its hash checked; otherwise only its segments are fetched.
///
/// Returns the byte range written: `range` clamped to the file and widened
/// to segment boundaries. File Provider partial fetches rely on this.
pub async fn download<S, W>(
    store: &S,
    content: &FileContent,
    key: &FileKey,
    range: Range<u64>,
    sink: Arc<W>,
    options: TransferOptions,
    progress: Progress<'_>,
) -> Result<Range<u64>>
where
    S: BlobStore,
    W: WriteAt + 'static,
{
    let layout = content.layout();
    let done = &AtomicU64::new(0);

    stream::iter(layout.chunks_covering(range.clone()))
        .map(|index| {
            let sink = Arc::clone(&sink);
            let key = key.clone();
            let chunk = content.chunks()[index as usize];
            let needed = layout.segments_covering(index, range.clone());
            async move {
                let chunk_len = layout.chunk_len(index);
                let data_end = (needed.end * SEGMENT_SIZE).min(chunk_len);
                let whole = needed.start == 0 && data_end == chunk_len;
                let (segments, blob) = if whole {
                    let all = 0..layout.segment_count(index);
                    (all, store.get(chunk.blob).await?)
                } else {
                    let blob_range = layout.segment_blob_range(index, needed.clone());
                    (
                        needed.clone(),
                        store.get_range(chunk.blob, blob_range).await?,
                    )
                };
                let offset = layout.chunk_range(index).start + (needed.start * SEGMENT_SIZE) as u64;
                let written = (data_end - needed.start * SEGMENT_SIZE) as u64;
                blocking(move || {
                    if whole && Sha256Hash::digest(&blob) != chunk.blob {
                        return Err(Error::BlobHashMismatch(chunk.blob));
                    }
                    let mut plaintext =
                        open_segments(&key, &layout, index, &chunk, segments, blob.into())?;
                    plaintext.truncate(written as usize);
                    sink.write_all_at(&plaintext, offset)?;
                    Ok::<_, Error>(())
                })
                .await?;
                progress(done.fetch_add(written, Ordering::Relaxed) + written);
                Ok::<_, Error>(())
            }
        })
        .buffer_unordered(options.concurrency.max(1))
        .try_collect::<()>()
        .await?;

    Ok(layout.segment_aligned(range))
}
