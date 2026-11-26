//! The user's key: on this device (an nsec), or in a remote signer reached
//! with NIP-46. A device needs it to sign in, that is to read or write a
//! drive descriptor, and keeps only the drive's keys afterwards.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use nostr::event::{AsyncSignEvent, Event, UnsignedEvent};
use nostr::key::{AsyncGetPublicKey, Keys, PublicKey};
use nostr::nips::nip44::AsyncNip44;
use nostr::nips::nip46::NostrConnectUri;
use nostr::types::{RelayUrl, Url};
use nostr_connect::client::{AuthUrlHandler, NostrConnect};

use crate::error::{Error, Result};

/// Where `nostrconnect://` pairings meet.
pub const NOSTR_CONNECT_RELAYS: [&str; 3] = [
    "wss://promenade.fiatjaf.com",
    "wss://relay.ditto.pub",
    "wss://relay.primal.net",
];
/// Each request may wait for the user to approve it in the signer.
const SIGNER_TIMEOUT: Duration = Duration::from_secs(120);
/// Covers finding the phone and scanning the code.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const APP_NAME: &str = "Lupine";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SignerError(String);

impl SignerError {
    fn new(error: impl fmt::Display) -> Self {
        Self(error.to_string())
    }
}

/// Opens the page where a remote signer asks the user to approve the app
/// (NIP-46 `auth_url`).
pub type OpenUrl = Arc<dyn Fn(Url) + Send + Sync>;

#[derive(Debug)]
pub enum UserKey {
    Local(Keys),
    Remote(Box<NostrConnect>),
}

impl UserKey {
    /// An nsec, a secret key in hex, or a `bunker://` link.
    pub fn parse(text: &str, open_url: Option<OpenUrl>) -> Result<Self> {
        let text = text.trim();
        if text.starts_with("bunker://") {
            let uri = NostrConnectUri::parse(text)
                .map_err(|_| Error::Malformed("invalid bunker:// link"))?;
            return remote(uri, Keys::generate(), SIGNER_TIMEOUT, open_url);
        }
        Keys::parse(text)
            .map(Self::Local)
            .map_err(|_| Error::Malformed("not an nsec or a bunker:// link"))
    }

    /// A `nostrconnect://` link for a signer to open, and the key it will
    /// give once the user accepts: the first request waits for that.
    pub fn pair(relays: Vec<RelayUrl>, open_url: Option<OpenUrl>) -> Result<(String, Self)> {
        let keys = Keys::generate();
        let uri = NostrConnectUri::client(keys.public_key(), relays, APP_NAME);
        let link = uri.to_string();
        Ok((link, remote(uri, keys, PAIRING_TIMEOUT, open_url)?))
    }

    pub async fn public_key(&self) -> Result<PublicKey> {
        self.get_public_key_async()
            .await
            .map_err(|e| Error::Nostr(e.to_string()))
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    pub async fn shutdown(&self) {
        if let Self::Remote(signer) = self {
            NostrConnect::clone(signer).shutdown().await;
        }
    }
}

fn remote(
    uri: NostrConnectUri,
    keys: Keys,
    timeout: Duration,
    open_url: Option<OpenUrl>,
) -> Result<UserKey> {
    crate::relays::install_tls_provider();
    let mut signer =
        NostrConnect::new(uri, keys, timeout, None).map_err(|e| Error::Nostr(e.to_string()))?;
    if let Some(open_url) = open_url {
        signer.auth_url_handler(Approval(open_url));
    }
    Ok(UserKey::Remote(Box::new(signer)))
}

struct Approval(OpenUrl);

impl fmt::Debug for Approval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Approval")
    }
}

impl AuthUrlHandler for Approval {
    fn on_auth_url(
        &self,
        auth_url: Url,
    ) -> Pin<Box<dyn Future<Output = Result<(), nostr_connect::error::Error>> + Send + '_>> {
        (self.0)(auth_url);
        Box::pin(async { Ok(()) })
    }
}

type Signed<'a, T> = Pin<Box<dyn Future<Output = Result<T, SignerError>> + Send + 'a>>;

impl AsyncGetPublicKey for UserKey {
    type Error = SignerError;

    fn get_public_key_async(&self) -> Signed<'_, PublicKey> {
        Box::pin(async move {
            match self {
                Self::Local(keys) => Ok(keys.public_key()),
                Self::Remote(signer) => signer
                    .get_public_key_async()
                    .await
                    .map_err(SignerError::new),
            }
        })
    }
}

impl AsyncSignEvent for UserKey {
    type Error = SignerError;

    fn sign_event_async(&self, unsigned: UnsignedEvent) -> Signed<'_, Event> {
        Box::pin(async move {
            match self {
                Self::Local(keys) => keys
                    .sign_event_async(unsigned)
                    .await
                    .map_err(SignerError::new),
                Self::Remote(signer) => signer
                    .sign_event_async(unsigned)
                    .await
                    .map_err(SignerError::new),
            }
        })
    }
}

impl AsyncNip44 for UserKey {
    type Error = SignerError;

    fn nip44_encrypt_async<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> Signed<'a, String> {
        Box::pin(async move {
            match self {
                Self::Local(keys) => keys
                    .nip44_encrypt_async(public_key, content)
                    .await
                    .map_err(SignerError::new),
                Self::Remote(signer) => signer
                    .nip44_encrypt_async(public_key, content)
                    .await
                    .map_err(SignerError::new),
            }
        })
    }

    fn nip44_decrypt_async<'a>(
        &'a self,
        public_key: &'a PublicKey,
        payload: &'a str,
    ) -> Signed<'a, String> {
        Box::pin(async move {
            match self {
                Self::Local(keys) => keys
                    .nip44_decrypt_async(public_key, payload)
                    .await
                    .map_err(SignerError::new),
                Self::Remote(signer) => signer
                    .nip44_decrypt_async(public_key, payload)
                    .await
                    .map_err(SignerError::new),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use nostr::nips::nip19::ToBech32;

    use super::*;

    #[tokio::test]
    async fn parses_nsec_and_hex_keys() {
        let keys = Keys::generate();
        for text in [
            keys.secret_key().to_bech32().unwrap(),
            format!("  {}\n", keys.secret_key().to_secret_hex()),
        ] {
            let parsed = UserKey::parse(&text, None).unwrap();
            assert!(!parsed.is_remote());
            assert_eq!(parsed.public_key().await.unwrap(), keys.public_key());
        }
        assert!(UserKey::parse("npub1xyz", None).is_err());
        assert!(UserKey::parse("bunker://not-a-key", None).is_err());
    }

    #[test]
    fn pairing_links_name_the_app_and_its_relays() {
        let relays = vec![RelayUrl::parse("wss://relay.example.com").unwrap()];
        let (link, key) = UserKey::pair(relays, None).unwrap();
        assert!(key.is_remote());
        let NostrConnectUri::Client {
            relays, metadata, ..
        } = NostrConnectUri::parse(&link).unwrap()
        else {
            panic!("not a nostrconnect:// link: {link}");
        };
        assert_eq!(metadata.name, APP_NAME);
        assert_eq!(relays.len(), 1);
    }
}
