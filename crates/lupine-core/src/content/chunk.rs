//! A chunk's blob is a run of XChaCha20-Poly1305 segments, each followed by
//! its tag. Nonces derive from the chunk's MAC, which the encrypted chunk
//! table already holds, so a reader can decrypt any segment without fetching
//! anything else.

use std::fmt;
use std::io;
use std::iter;
use std::ops::Range;

use bytes::Bytes;
use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{Tag, XNonce};

use super::layout::{ChunkLayout, SEGMENT_SIZE};
use crate::crypto::{NONCE_LEN, SymmetricKey, TAG_LEN};
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

const SEALED_SEGMENT: usize = SEGMENT_SIZE + TAG_LEN;
const NONCE_PREFIX_LEN: usize = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRef {
    /// Blossom address of the encrypted chunk.
    pub blob: Sha256Hash,
    /// Keyed hash of the chunk's index and plaintext. Unchanged chunks keep
    /// their blob, and the segment nonces derive from it.
    pub mac: [u8; 32],
}

/// Per-file content key. Only `root` is stored, in the node versions.
#[derive(Clone)]
pub struct FileKey {
    root: SymmetricKey,
    enc: SymmetricKey,
    mac: SymmetricKey,
    nonce: SymmetricKey,
    table: SymmetricKey,
}

impl FileKey {
    pub fn generate() -> Self {
        Self::from_root(SymmetricKey::generate())
    }

    pub fn from_root(root: SymmetricKey) -> Self {
        Self {
            enc: root.derive(b"lupine/v1/file/enc"),
            mac: root.derive(b"lupine/v1/file/mac"),
            nonce: root.derive(b"lupine/v1/file/nonce"),
            table: root.derive(b"lupine/v1/file/table"),
            root,
        }
    }

    pub fn root(&self) -> &SymmetricKey {
        &self.root
    }

    pub(crate) fn enc(&self) -> &SymmetricKey {
        &self.enc
    }

    pub(crate) fn table(&self) -> &SymmetricKey {
        &self.table
    }

    /// Deterministic like the segment nonces, so a table sealed again after
    /// a crash is the same blob.
    pub(crate) fn table_nonce(&self, table: &[u8]) -> [u8; NONCE_LEN] {
        self.nonce.mac_parts([&b"lupine/v1/table"[..], table])[..NONCE_LEN]
            .try_into()
            .expect("nonce length")
    }

    pub(crate) fn chunk_mac<'a>(
        &self,
        index: u64,
        plaintext: impl IntoIterator<Item = &'a [u8]>,
    ) -> [u8; 32] {
        let position = index.to_be_bytes();
        let plaintext = plaintext.into_iter().map(|part| -> &[u8] { part });
        self.mac
            .mac_parts(iter::once(&position[..]).chain(plaintext))
    }

    /// Deterministic: re-encrypting the same chunk after a crash yields the
    /// same blob. Unique per (index, plaintext), since the MAC covers both.
    fn nonce_prefix(&self, mac: &[u8; 32]) -> [u8; NONCE_PREFIX_LEN] {
        self.nonce.mac(mac)[..NONCE_PREFIX_LEN]
            .try_into()
            .expect("prefix length")
    }
}

impl fmt::Debug for FileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileKey(<redacted>)")
    }
}

fn segment_nonce(prefix: &[u8; NONCE_PREFIX_LEN], segment: usize, last: bool) -> XNonce {
    let mut nonce = [0u8; 24];
    nonce[..NONCE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[NONCE_PREFIX_LEN..23].copy_from_slice(&(segment as u32).to_be_bytes());
    nonce[23] = u8::from(last);
    XNonce::from(nonce)
}

/// Binds every segment to its chunk's position in the file.
fn segment_aad(index: u64) -> [u8; 25] {
    let mut aad = [0u8; 25];
    aad[..17].copy_from_slice(b"lupine/v1/segment");
    aad[17..].copy_from_slice(&index.to_be_bytes());
    aad
}

fn segment_len(padded_len: usize, segment: usize) -> usize {
    (padded_len - segment * SEGMENT_SIZE).min(SEGMENT_SIZE)
}

/// A chunk laid out as its future blob: each segment's plaintext followed by
/// room for its tag, so it is encrypted in place, in a single allocation.
pub(crate) struct ChunkBuffer {
    buf: Vec<u8>,
    padded_len: usize,
}

impl ChunkBuffer {
    /// Starts zeroed, which doubles as padding.
    pub(crate) fn new(padded_len: usize) -> Self {
        let segments = padded_len.div_ceil(SEGMENT_SIZE);
        Self {
            buf: vec![0; padded_len + segments * TAG_LEN],
            padded_len,
        }
    }

    /// Fills the first `len` plaintext bytes, segment by segment. `read`
    /// receives a slot and its offset in the chunk.
    pub(crate) fn fill(
        &mut self,
        len: usize,
        mut read: impl FnMut(&mut [u8], u64) -> io::Result<()>,
    ) -> io::Result<()> {
        for (segment, sealed) in self.buf.chunks_mut(SEALED_SEGMENT).enumerate() {
            let start = segment * SEGMENT_SIZE;
            if start >= len {
                break;
            }
            read(&mut sealed[..(len - start).min(SEGMENT_SIZE)], start as u64)?;
        }
        Ok(())
    }

    /// The first `len` plaintext bytes, segment by segment.
    pub(crate) fn plaintext(&self, len: usize) -> impl Iterator<Item = &[u8]> {
        self.buf
            .chunks(SEALED_SEGMENT)
            .enumerate()
            .map_while(move |(segment, sealed)| {
                let start = segment * SEGMENT_SIZE;
                (start < len).then(|| &sealed[..(len - start).min(SEGMENT_SIZE)])
            })
    }

    pub(crate) fn seal(mut self, key: &FileKey, index: u64, mac: &[u8; 32]) -> (Sha256Hash, Bytes) {
        let prefix = key.nonce_prefix(mac);
        let aad = segment_aad(index);
        let cipher = key.enc().cipher();
        let count = self.padded_len.div_ceil(SEGMENT_SIZE);
        for (segment, sealed) in self.buf.chunks_mut(SEALED_SEGMENT).enumerate() {
            let (data, tag) = sealed.split_at_mut(segment_len(self.padded_len, segment));
            let nonce = segment_nonce(&prefix, segment, segment + 1 == count);
            let computed = cipher
                .encrypt_inout_detached(&nonce, &aad, data.into())
                .expect("segments are far below the XChaCha20 limit");
            tag.copy_from_slice(&computed);
        }
        (Sha256Hash::digest(&self.buf), Bytes::from(self.buf))
    }
}

/// Decrypts `segments` of chunk `index`, given the blob bytes holding exactly
/// those segments. Returns their plaintext, padding included.
pub(crate) fn open_segments(
    key: &FileKey,
    layout: &ChunkLayout,
    index: u64,
    chunk: &ChunkRef,
    segments: Range<usize>,
    mut sealed: Vec<u8>,
) -> Result<Vec<u8>> {
    let expected = layout.segment_blob_range(index, segments.clone());
    if sealed.len() as u64 != expected.end - expected.start {
        return Err(Error::Malformed("segment range has the wrong length"));
    }
    let prefix = key.nonce_prefix(&chunk.mac);
    let aad = segment_aad(index);
    let cipher = key.enc().cipher();
    let padded_len = layout.padded_len(index);
    let count = layout.segment_count(index);
    let (mut read, mut write) = (0, 0);
    for segment in segments {
        let len = segment_len(padded_len, segment);
        let tag: [u8; TAG_LEN] = sealed[read + len..read + len + TAG_LEN]
            .try_into()
            .expect("tag length");
        let nonce = segment_nonce(&prefix, segment, segment + 1 == count);
        cipher
            .decrypt_inout_detached(
                &nonce,
                &aad,
                (&mut sealed[read..read + len]).into(),
                &Tag::from(tag),
            )
            .map_err(|_| Error::Decrypt)?;
        sealed.copy_within(read..read + len, write);
        read += len + TAG_LEN;
        write += len;
    }
    sealed.truncate(write);
    Ok(sealed)
}
