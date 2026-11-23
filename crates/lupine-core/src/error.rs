use crate::ids::Sha256Hash;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("decryption failed: wrong key or tampered data")]
    Decrypt,
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u8),
    #[error("malformed data: {0}")]
    Malformed(&'static str),
    #[error("blob {0} does not match its hash")]
    BlobHashMismatch(Sha256Hash),
    #[error("blob {0} not found")]
    BlobNotFound(Sha256Hash),
    #[error("blob store error: {0}")]
    Store(String),
    #[error("the source changed while it was being read")]
    SourceChanged,
    #[error("nostr: {0}")]
    Nostr(String),
    #[error("invalid chunk size {0}")]
    InvalidChunkSize(u32),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
