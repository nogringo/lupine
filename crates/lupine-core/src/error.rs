use crate::ids::Sha256Hash;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("decryption failed: wrong key or tampered data")]
    Decrypt,
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u8),
    #[error("malformed data: {0}")]
    Malformed(&'static str),
    #[error("invalid name: 1 to 255 UTF-8 bytes, not . or .., no / or NUL")]
    InvalidName,
    #[error("no such node")]
    UnknownNode,
    /// The parent may only be late.
    #[error("no such parent folder")]
    UnknownParent,
    #[error("the folder is not empty")]
    FolderNotEmpty,
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
    #[error("{relay}: {message}")]
    Relay { relay: String, message: String },
    /// No drive relay accepted the event.
    #[error("no relay accepted the event: {0}")]
    NotPublished(String),
    #[error("invalid chunk size {0}")]
    InvalidChunkSize(u32),
    /// The file system integration should enumerate everything again.
    #[error("sync anchor expired")]
    AnchorExpired,
    #[error("local database: {0}")]
    Database(#[from] rusqlite::Error),
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
            Self::Network { .. } | Self::Relay { .. } | Self::NotPublished(_) => true,
            Self::Http { status, .. } => matches!(status, 408 | 429 | 500 | 502 | 503 | 504),
            _ => false,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
