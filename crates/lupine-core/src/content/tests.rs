use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;

use super::*;
use crate::blob::{BlobStore, DirBlobStore, MemoryBlobStore};
use crate::error::{Error, Result};
use crate::io::MemoryFile;

/// One segment per chunk.
const CS: u32 = MIN_CHUNK_SIZE;
const STEP: usize = CS as usize;
/// Sixteen segments per chunk.
const BIG: u32 = 1 << 20;
const SEG: usize = SEGMENT_SIZE;
const SEALED_SEG: usize = SEG + 16;

fn options(chunk_size: u32) -> TransferOptions {
    TransferOptions {
        chunk_size,
        concurrency: 3,
    }
}

fn data(len: usize, seed: u8) -> Vec<u8> {
    let mut state = u32::from(seed) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

async fn upload_bytes<S: BlobStore>(
    store: &S,
    key: &FileKey,
    bytes: &[u8],
    chunk_size: u32,
) -> FileContent {
    let source = Arc::new(MemoryFile::new(bytes.to_vec()));
    upload(store, source, key, None, options(chunk_size), &|_| {})
        .await
        .unwrap()
}

async fn read<S: BlobStore>(
    store: &S,
    content: &FileContent,
    key: &FileKey,
    range: Range<u64>,
) -> Result<(Range<u64>, Vec<u8>)> {
    let sink = Arc::new(MemoryFile::default());
    let options = options(content.layout().chunk_size());
    let written = download(
        store,
        content,
        key,
        range,
        Arc::clone(&sink),
        options,
        &|_| {},
    )
    .await?;
    Ok((written, sink.snapshot()))
}

#[tokio::test]
async fn round_trip_at_chunk_boundaries() {
    for chunk_size in [CS, BIG] {
        let step = chunk_size as usize;
        for len in [
            0,
            1,
            SEG - 1,
            SEG + 1,
            step - 1,
            step,
            step + 1,
            3 * step + step / 2,
        ] {
            let store = MemoryBlobStore::new();
            let key = FileKey::generate();
            let bytes = data(len, len as u8);
            let content = upload_bytes(&store, &key, &bytes, chunk_size).await;
            assert_eq!(content.chunks().len(), len.div_ceil(step));
            let (written, out) = read(&store, &content, &key, 0..u64::MAX).await.unwrap();
            assert_eq!(written, 0..len as u64);
            assert_eq!(out, bytes, "chunk size {chunk_size}, len {len}");
        }
    }
}

#[tokio::test]
async fn blob_sizes_hide_the_exact_length() {
    let store = MemoryBlobStore::new();
    upload_bytes(&store, &FileKey::generate(), &data(STEP + 1000, 1), CS).await;
    let mut lens = store.blob_lens();
    lens.sort();
    assert_eq!(lens, vec![4096 + 16, STEP + 16]);
}

#[tokio::test]
async fn a_seek_fetches_one_segment() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let step = BIG as usize;
    let bytes = data(3 * step + 100, 2);
    let content = upload_bytes(&store, &key, &bytes, BIG).await;

    let at = (step + SEG + 4464) as u64;
    let before = store.bytes_served();
    let (written, out) = read(&store, &content, &key, at..at + 10).await.unwrap();
    assert_eq!(store.bytes_served() - before, SEALED_SEG);
    let segment = step + SEG..step + 2 * SEG;
    assert_eq!(written, segment.start as u64..segment.end as u64);
    assert_eq!(&out[segment.clone()], &bytes[segment.clone()]);
    assert!(out[..segment.start].iter().all(|&b| b == 0));
}

#[tokio::test]
async fn a_range_across_chunks_fetches_their_edge_segments() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let step = BIG as usize;
    let bytes = data(2 * step, 3);
    let content = upload_bytes(&store, &key, &bytes, BIG).await;

    let before = store.bytes_served();
    let (written, out) = read(&store, &content, &key, step as u64 - 10..step as u64 + 10)
        .await
        .unwrap();
    assert_eq!(store.bytes_served() - before, 2 * SEALED_SEG);
    let expected = step - SEG..step + SEG;
    assert_eq!(written, expected.start as u64..expected.end as u64);
    assert_eq!(&out[expected.clone()], &bytes[expected]);
}

#[tokio::test]
async fn partial_reads_in_a_padded_last_chunk() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let step = BIG as usize;
    let bytes = data(step + 70_000, 4);
    let content = upload_bytes(&store, &key, &bytes, BIG).await;
    assert_eq!(content.layout().padded_len(1), 71_680);

    let end = bytes.len() as u64;
    let before = store.bytes_served();
    let (written, out) = read(&store, &content, &key, end - 10..end).await.unwrap();
    assert_eq!(store.bytes_served() - before, 71_680 - SEG + 16);
    assert_eq!(written, (step + SEG) as u64..end);
    assert_eq!(&out[step + SEG..], &bytes[step + SEG..]);
}

#[tokio::test]
async fn reupload_only_sends_changed_chunks() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let mut bytes = data(5 * STEP, 3);
    let first = upload_bytes(&store, &key, &bytes, CS).await;
    assert_eq!(store.put_count(), 5);

    bytes[2 * STEP + 17] ^= 0xff;
    let source = Arc::new(MemoryFile::new(bytes.clone()));
    let second = upload(&store, source, &key, Some(&first), options(CS), &|_| {})
        .await
        .unwrap();
    assert_eq!(store.put_count(), 6);
    for i in [0, 1, 3, 4] {
        assert_eq!(first.chunks()[i], second.chunks()[i]);
    }
    assert_ne!(first.chunks()[2], second.chunks()[2]);
    assert_eq!(
        read(&store, &second, &key, 0..u64::MAX).await.unwrap().1,
        bytes
    );
}

#[tokio::test]
async fn encrypting_again_gives_the_same_blobs() {
    let key = FileKey::generate();
    let bytes = data(2 * STEP + 5, 5);
    let first = upload_bytes(&MemoryBlobStore::new(), &key, &bytes, CS).await;
    let second = upload_bytes(&MemoryBlobStore::new(), &key, &bytes, CS).await;
    assert_eq!(first, second);
}

#[tokio::test]
async fn identical_chunks_get_unrelated_blobs() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let bytes = data(STEP, 6).repeat(2);
    let content = upload_bytes(&store, &key, &bytes, CS).await;
    let [a, b] = content.chunks() else { panic!() };
    assert_ne!(a.blob, b.blob);
    let a = store.get(a.blob).await.unwrap();
    let b = store.get(b.blob).await.unwrap();
    let same = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    assert!(
        same < STEP / 64,
        "ciphertexts share a keystream: {same} equal bytes"
    );
}

#[tokio::test]
async fn whole_chunk_reads_check_the_hash() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let content = upload_bytes(&store, &key, &data(2 * STEP, 4), CS).await;
    let victim = content.chunks()[1].blob;
    let mut corrupt = store.get(victim).await.unwrap().to_vec();
    corrupt[100] ^= 1;
    store.overwrite(victim, Bytes::from(corrupt));

    let result = read(&store, &content, &key, 0..u64::MAX).await;
    assert!(matches!(result, Err(Error::BlobHashMismatch(hash)) if hash == victim));
}

#[tokio::test]
async fn tampered_segments_fail_partial_reads() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let bytes = data(BIG as usize, 7);
    let content = upload_bytes(&store, &key, &bytes, BIG).await;
    let blob = content.chunks()[0].blob;
    let mut corrupt = store.get(blob).await.unwrap().to_vec();
    corrupt[SEALED_SEG + 100] ^= 1;
    store.overwrite(blob, Bytes::from(corrupt));

    let in_segment = |s: usize| (s * SEG) as u64..(s * SEG + 1) as u64;
    assert!(matches!(
        read(&store, &content, &key, in_segment(1)).await,
        Err(Error::Decrypt)
    ));
    let (_, out) = read(&store, &content, &key, in_segment(0)).await.unwrap();
    assert_eq!(&out[..SEG], &bytes[..SEG]);
}

#[tokio::test]
async fn swapped_chunks_are_rejected() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();
    let content = upload_bytes(&store, &key, &data(2 * STEP, 5), CS).await;
    let mut chunks = content.chunks().to_vec();
    chunks.swap(0, 1);
    let swapped = FileContent::new(content.layout(), chunks).unwrap();
    assert!(matches!(
        read(&store, &swapped, &key, 0..STEP as u64).await,
        Err(Error::Decrypt)
    ));
}

#[tokio::test]
async fn wrong_key_is_rejected() {
    let store = MemoryBlobStore::new();
    let content = upload_bytes(&store, &FileKey::generate(), &data(100, 6), CS).await;
    assert!(matches!(
        read(&store, &content, &FileKey::generate(), 0..100).await,
        Err(Error::Decrypt)
    ));
}

#[tokio::test]
async fn small_tables_are_inlined_and_large_ones_stored() {
    let store = MemoryBlobStore::new();
    let key = FileKey::generate();

    let small = upload_bytes(&store, &key, &data(3 * STEP, 7), CS).await;
    let descriptor = small.describe(&store, &key).await.unwrap();
    assert!(descriptor.chunks.is_some() && descriptor.table.is_none());
    assert_eq!(descriptor.resolve(&store, &key).await.unwrap(), small);

    let bytes = data((INLINE_CHUNK_LIMIT + 1) * STEP, 8);
    let large = upload_bytes(&store, &key, &bytes, CS).await;
    let descriptor = large.describe(&store, &key).await.unwrap();
    assert!(descriptor.chunks.is_none() && descriptor.table.is_some());
    assert_eq!(descriptor.resolve(&store, &key).await.unwrap(), large);
    let again = large.describe(&store, &key).await.unwrap();
    assert_eq!(
        again.table, descriptor.table,
        "a table sealed again is the same blob"
    );

    let json = serde_json::to_string(&descriptor).unwrap();
    let parsed: ContentDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, descriptor);
    assert!(parsed.resolve(&store, &FileKey::generate()).await.is_err());
}

#[tokio::test]
async fn progress_reaches_the_file_size() {
    let store = MemoryBlobStore::new();
    let bytes = data(3 * STEP + 5, 10);
    let last = AtomicU64::new(0);
    let source = Arc::new(MemoryFile::new(bytes.clone()));
    let record = |done| {
        last.fetch_max(done, Ordering::Relaxed);
    };
    upload(
        &store,
        source,
        &FileKey::generate(),
        None,
        options(CS),
        &record,
    )
    .await
    .unwrap();
    assert_eq!(last.load(Ordering::Relaxed), bytes.len() as u64);
}

#[tokio::test]
async fn files_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirBlobStore::new(dir.path().join("blobs")).unwrap();
    let key = FileKey::generate();
    let bytes = data(2 * BIG as usize + 3, 11);

    let input_path = dir.path().join("input.bin");
    std::fs::write(&input_path, &bytes).unwrap();
    let source = Arc::new(std::fs::File::open(&input_path).unwrap());
    let content = upload(&store, source, &key, None, options(BIG), &|_| {})
        .await
        .unwrap();

    let output_path = dir.path().join("output.bin");
    let sink = Arc::new(std::fs::File::create(&output_path).unwrap());
    download(
        &store,
        &content,
        &key,
        0..u64::MAX,
        sink,
        options(BIG),
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&output_path).unwrap(), bytes);

    let at = BIG as u64 + 3 * SEG as u64 + 7;
    let (_, out) = read(&store, &content, &key, at..at + 1).await.unwrap();
    assert_eq!(out[at as usize], bytes[at as usize]);
}
