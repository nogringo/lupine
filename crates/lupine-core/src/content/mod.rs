//! File content: fixed-size chunks, each encrypted into its own blob.

mod chunk;
mod layout;
mod table;
mod transfer;

pub use chunk::{ChunkRef, FileKey};
pub use layout::{ChunkLayout, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE, SEGMENT_SIZE};
pub use table::{ContentDescriptor, FileContent, INLINE_CHUNK_LIMIT};
pub use transfer::{Progress, TransferOptions, download, upload};

#[cfg(test)]
mod tests;
