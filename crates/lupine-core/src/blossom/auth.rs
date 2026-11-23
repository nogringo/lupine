//! BUD-11 authorization tokens, signed by the drive key. Each token is
//! scoped to one server and one blob.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::types::Timestamp;

use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

/// Covers a slow chunk upload: servers check the token when the request
/// starts or when its body ends.
const LIFETIME_SECS: u64 = 15 * 60;
/// Servers reject tokens dated in the future, so leave room for clock skew.
const BACKDATE_SECS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verb {
    Get,
    Upload,
    Delete,
}

impl Verb {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Upload => "upload",
            Self::Delete => "delete",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Get => "Get blob",
            Self::Upload => "Upload blob",
            Self::Delete => "Delete blob",
        }
    }
}

/// Value of the `Authorization` header.
pub(crate) fn header(keys: &Keys, verb: Verb, domain: &str, hash: &Sha256Hash) -> Result<String> {
    let now = Timestamp::now().as_secs();
    let tag = |name: &str, value: String| Tag::parse([name.to_owned(), value]);
    let tags = [
        tag("t", verb.as_str().to_owned()),
        tag("server", domain.to_owned()),
        tag("x", hash.to_hex()),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| Error::Nostr(e.to_string()))?;
    let event = EventBuilder::new(Kind::BlossomAuth, verb.description())
        .tags(tags)
        .tag(Tag::expiration(Timestamp::from_secs(now + LIFETIME_SECS)))
        .custom_created_at(Timestamp::from_secs(now.saturating_sub(BACKDATE_SECS)))
        .finalize(keys)
        .map_err(|e| Error::Nostr(e.to_string()))?;
    Ok(format!("Nostr {}", URL_SAFE_NO_PAD.encode(event.as_json())))
}
