use serde::{Deserialize, Serialize};

use super::chunk::{ChunkRef, FileKey};
use super::layout::ChunkLayout;
use crate::blob::BlobStore;
use crate::crypto::{open, seal};
use crate::encoding;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

/// Above this many chunks (512 MiB at the default chunk size) the chunk
/// table moves out of the node event into its own encrypted blob, which
/// keeps node events well under relay size limits.
pub const INLINE_CHUNK_LIMIT: usize = 64;
const ENTRY_LEN: usize = 64;
const TABLE_AAD: &[u8] = b"lupine/v1/table";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileContent {
    layout: ChunkLayout,
    chunks: Vec<ChunkRef>,
}

impl FileContent {
    pub fn new(layout: ChunkLayout, chunks: Vec<ChunkRef>) -> Result<Self> {
        if chunks.len() as u64 != layout.chunk_count() {
            return Err(Error::Malformed("chunk count does not match the file size"));
        }
        Ok(Self { layout, chunks })
    }

    pub fn layout(&self) -> ChunkLayout {
        self.layout
    }

    pub fn chunks(&self) -> &[ChunkRef] {
        &self.chunks
    }

    /// Stores the chunk table if it is too large to inline.
    pub async fn describe<S: BlobStore>(
        &self,
        store: &S,
        key: &FileKey,
    ) -> Result<ContentDescriptor> {
        let table = self.encode_table();
        let (chunks, table) = if self.chunks.len() <= INLINE_CHUNK_LIMIT {
            (Some(table), None)
        } else {
            let blob = seal(key.table(), &key.table_nonce(&table), TABLE_AAD, &table);
            let hash = Sha256Hash::digest(&blob);
            store.put(hash, blob.into()).await?;
            (None, Some(hash))
        };
        Ok(ContentDescriptor {
            size: self.layout.size(),
            chunk_size: self.layout.chunk_size(),
            chunks,
            table,
        })
    }

    fn encode_table(&self) -> Vec<u8> {
        let mut table = Vec::with_capacity(self.chunks.len() * ENTRY_LEN);
        for chunk in &self.chunks {
            table.extend_from_slice(chunk.blob.as_bytes());
            table.extend_from_slice(&chunk.mac);
        }
        table
    }

    fn decode_table(layout: ChunkLayout, table: &[u8]) -> Result<Self> {
        let (entries, rest) = table.as_chunks::<ENTRY_LEN>();
        if !rest.is_empty() {
            return Err(Error::Malformed("chunk table length"));
        }
        let chunks = entries
            .iter()
            .map(|entry| ChunkRef {
                blob: Sha256Hash(entry[..32].try_into().expect("entry length")),
                mac: entry[32..].try_into().expect("entry length"),
            })
            .collect();
        Self::new(layout, chunks)
    }
}

/// How a file's content appears inside its node metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDescriptor {
    pub size: u64,
    pub chunk_size: u32,
    /// Inline chunk table, base64.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "encoding::base64_opt"
    )]
    pub chunks: Option<Vec<u8>>,
    /// Address of the encrypted chunk table blob, for large files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<Sha256Hash>,
}

impl ContentDescriptor {
    pub async fn resolve<S: BlobStore>(&self, store: &S, key: &FileKey) -> Result<FileContent> {
        let layout = ChunkLayout::new(self.size, self.chunk_size)?;
        let table = match (&self.chunks, self.table) {
            (Some(inline), None) => inline.clone(),
            (None, Some(hash)) => {
                let blob = store.get(hash).await?;
                if Sha256Hash::digest(&blob) != hash {
                    return Err(Error::BlobHashMismatch(hash));
                }
                open(key.table(), TABLE_AAD, blob.into())?
            }
            _ => {
                return Err(Error::Malformed(
                    "content needs exactly one of chunks or table",
                ));
            }
        };
        FileContent::decode_table(layout, &table)
    }
}
