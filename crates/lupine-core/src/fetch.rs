//! File content into local files, whole or in part, the way file system
//! integrations ask for it.

use std::collections::VecDeque;
use std::ops::Range;
#[cfg(any(unix, windows))]
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::blob::BlobStore;
use crate::content::{self, FileContent, Progress, TransferOptions};
use crate::error::Result;
use crate::ids::Sha256Hash;
use crate::io::WriteAt;
use crate::node::FileMeta;
#[cfg(any(unix, windows))]
use crate::task::blocking;

/// External chunk tables kept, so reads that seek through a large file do
/// not fetch its table each time.
const CACHED_TABLES: usize = 16;

pub struct FileReader<S> {
    blobs: S,
    options: TransferOptions,
    tables: Mutex<VecDeque<(Sha256Hash, Arc<FileContent>)>>,
}

impl<S: BlobStore> FileReader<S> {
    pub fn new(blobs: S) -> Self {
        Self {
            blobs,
            options: TransferOptions::default(),
            tables: Mutex::new(VecDeque::new()),
        }
    }

    pub fn blobs(&self) -> &S {
        &self.blobs
    }

    /// Writes the content of `file` covering `range` into a new file at
    /// `path`, at the same offsets, and returns the range written: `range`
    /// within the file, widened to `alignment` (a power of two) and to
    /// segment boundaries, except at the end of the file. The file at
    /// `path` has the full size; what lies outside that range stays sparse.
    #[cfg(any(unix, windows))]
    pub async fn read(
        &self,
        file: &FileMeta,
        range: Range<u64>,
        alignment: u64,
        path: &Path,
        progress: Progress<'_>,
    ) -> Result<Range<u64>> {
        let size = file.content.size;
        let content = self.content(file).await?;
        let path = path.to_owned();
        let sink = blocking(move || {
            let sink = std::fs::File::create(path)?;
            sink.set_len(size)?;
            Ok::<_, std::io::Error>(sink)
        })
        .await?;
        self.download(file, &content, range, alignment, Arc::new(sink), progress)
            .await
    }

    /// Like `read`, at the same offsets in `sink`.
    pub async fn read_into<W: WriteAt + 'static>(
        &self,
        file: &FileMeta,
        range: Range<u64>,
        alignment: u64,
        sink: Arc<W>,
        progress: Progress<'_>,
    ) -> Result<Range<u64>> {
        let content = self.content(file).await?;
        self.download(file, &content, range, alignment, sink, progress)
            .await
    }

    async fn download<W: WriteAt + 'static>(
        &self,
        file: &FileMeta,
        content: &FileContent,
        range: Range<u64>,
        alignment: u64,
        sink: Arc<W>,
        progress: Progress<'_>,
    ) -> Result<Range<u64>> {
        let size = file.content.size;
        let alignment = alignment.max(1).next_power_of_two();
        let start = range.start.min(size) / alignment * alignment;
        let end = range
            .end
            .min(size)
            .div_ceil(alignment)
            .saturating_mul(alignment)
            .min(size);
        if start >= end {
            return Ok(start..start);
        }
        content::download(
            &self.blobs,
            content,
            &file.file_key(),
            start..end,
            sink,
            self.options,
            progress,
        )
        .await
    }

    async fn content(&self, file: &FileMeta) -> Result<Arc<FileContent>> {
        let Some(table) = file.content.table else {
            let content = file.content.resolve(&self.blobs, &file.file_key()).await?;
            return Ok(Arc::new(content));
        };
        if let Some(content) = self.cached(&table) {
            return Ok(content);
        }
        let content = Arc::new(file.content.resolve(&self.blobs, &file.file_key()).await?);
        let mut tables = self.tables.lock().unwrap_or_else(|e| e.into_inner());
        if tables.len() == CACHED_TABLES {
            tables.pop_front();
        }
        tables.push_back((table, Arc::clone(&content)));
        Ok(content)
    }

    fn cached(&self, table: &Sha256Hash) -> Option<Arc<FileContent>> {
        let tables = self.tables.lock().unwrap_or_else(|e| e.into_inner());
        tables
            .iter()
            .find(|(hash, _)| hash == table)
            .map(|(_, content)| Arc::clone(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::MemoryBlobStore;
    use crate::content::{FileKey, MIN_CHUNK_SIZE, SEGMENT_SIZE};
    use crate::io::MemoryFile;

    const SEG: u64 = SEGMENT_SIZE as u64;

    async fn stored(
        len: usize,
        chunk_size: u32,
    ) -> (FileReader<MemoryBlobStore>, FileMeta, Vec<u8>) {
        let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let store = MemoryBlobStore::new();
        let key = FileKey::generate();
        let options = TransferOptions {
            chunk_size,
            concurrency: 4,
        };
        let source = Arc::new(MemoryFile::new(bytes.clone()));
        let uploaded = content::upload(&store, source, &key, None, options, &|_| {})
            .await
            .unwrap();
        let descriptor = uploaded.describe(&store, &key).await.unwrap();
        let file = FileMeta {
            key: key.root().clone(),
            content: descriptor,
        };
        (FileReader::new(store), file, bytes)
    }

    #[tokio::test]
    async fn partial_reads_land_at_their_offsets() {
        let len = 3 * MIN_CHUNK_SIZE as usize + 1000;
        let (reader, file, bytes) = stored(len, MIN_CHUNK_SIZE).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial");

        let written = reader
            .read(&file, SEG + 10..SEG + 20, 4096, &path, &|_| {})
            .await
            .unwrap();
        assert_eq!(written, SEG..2 * SEG);
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk.len(), len, "full size, sparse elsewhere");
        let range = written.start as usize..written.end as usize;
        assert_eq!(on_disk[range.clone()], bytes[range]);

        let tail = reader
            .read(&file, len as u64 - 1..len as u64, 1 << 20, &path, &|_| {})
            .await
            .unwrap();
        assert_eq!(tail, 0..len as u64, "widened to the alignment");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn large_files_fetch_their_table_once() {
        let chunks = crate::content::INLINE_CHUNK_LIMIT + 1;
        let (reader, file, bytes) = stored(chunks * MIN_CHUNK_SIZE as usize, MIN_CHUNK_SIZE).await;
        assert!(file.content.table.is_some());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large");
        for start in [0, 10 * SEG, 20 * SEG] {
            reader
                .read(&file, start..start + 1, 1, &path, &|_| {})
                .await
                .unwrap();
        }
        let served = reader.blobs().bytes_served();
        let table_len = chunks * 64 + crate::crypto::ENVELOPE_OVERHEAD;
        let segment_len = SEGMENT_SIZE + crate::crypto::TAG_LEN;
        assert_eq!(served, table_len + 3 * segment_len);

        let whole = reader
            .read(&file, 0..u64::MAX, 4096, &path, &|_| {})
            .await
            .unwrap();
        assert_eq!(whole, 0..bytes.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
