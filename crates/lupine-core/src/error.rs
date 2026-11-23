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
    /// `reason` is the server's `X-Reason` header, for display only.
    #[error("{server} answered HTTP {status}{}", .reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default())]
    Http {
        server: String,
        status: u16,
        reason: Option<String>,
    },
    #[error("{server} unreachable: {message}")]
    Network { server: String, message: String },
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

impl Error {
    /// The same request may succeed later: the network failed, or the
    /// server is overloaded or down.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network { .. } => true,
            Self::Http { status, .. } => matches!(status, 408 | 429 | 500 | 502 | 503 | 504),
            _ => false,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
