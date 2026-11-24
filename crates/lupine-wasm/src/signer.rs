//! The user's key on the web: an nsec, a NIP-46 signer, or a NIP-07
//! extension. Workers have no `window.nostr`, so the page relays the
//! extension's calls.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use js_sys::{Array, Function, JSON, Promise};
use lupine_core::identity::UserKey;
use nostr::event::{AsyncSignEvent, Event, UnsignedEvent};
use nostr::key::{AsyncGetPublicKey, PublicKey};
use nostr::nips::nip44::AsyncNip44;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::error::describe;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SignerError(String);

impl SignerError {
    fn new(error: impl fmt::Display) -> Self {
        Self(error.to_string())
    }
}

#[derive(Debug)]
pub enum WebSigner {
    Key(UserKey),
    Extension(Extension),
}

impl WebSigner {
    /// The user approves requests outside this page.
    pub(crate) fn is_remote(&self) -> bool {
        match self {
            Self::Key(key) => key.is_remote(),
            Self::Extension(_) => true,
        }
    }

    pub(crate) async fn shutdown(&self) {
        if let Self::Key(key) = self {
            key.shutdown().await;
        }
    }
}

/// `call(method, params)` runs `window.nostr[method](...params)` in the
/// page, `nip44.encrypt` and `nip44.decrypt` included, and resolves to what
/// it returns.
pub struct Extension {
    call: SendWrapper<Function>,
}

impl fmt::Debug for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Extension")
    }
}

impl Extension {
    pub(crate) fn new(call: Function) -> Self {
        Self {
            call: SendWrapper::new(call),
        }
    }

    async fn request(&self, method: &str, params: &[JsValue]) -> Result<JsValue, SignerError> {
        let params: Array = params.iter().collect();
        let returned = self
            .call
            .call2(&JsValue::NULL, &method.into(), &params)
            .map_err(|e| SignerError(describe(&e)))?;
        JsFuture::from(Promise::resolve(&returned))
            .await
            .map_err(|e| SignerError(describe(&e)))
    }

    async fn text(&self, method: &str, params: &[JsValue]) -> Result<String, SignerError> {
        self.request(method, params)
            .await?
            .as_string()
            .ok_or_else(|| SignerError(format!("{method} returned no text")))
    }

    async fn public_key(&self) -> Result<PublicKey, SignerError> {
        PublicKey::parse(&self.text("getPublicKey", &[]).await?).map_err(SignerError::new)
    }

    async fn sign(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        let template = JSON::parse(&unsigned.as_json()).map_err(|e| SignerError(describe(&e)))?;
        let signed = self.request("signEvent", &[template]).await?;
        let json = JSON::stringify(&signed)
            .ok()
            .and_then(|json| json.as_string())
            .ok_or_else(|| SignerError("signEvent returned no event".into()))?;
        let event = Event::from_json(json).map_err(SignerError::new)?;
        event.verify().map_err(SignerError::new)?;
        if event.pubkey != unsigned.pubkey || event.kind != unsigned.kind {
            return Err(SignerError("the extension signed another event".into()));
        }
        Ok(event)
    }
}

type Signed<'a, T> = Pin<Box<dyn Future<Output = Result<T, SignerError>> + Send + 'a>>;

impl AsyncGetPublicKey for WebSigner {
    type Error = SignerError;

    fn get_public_key_async(&self) -> Signed<'_, PublicKey> {
        Box::pin(SendWrapper::new(async move {
            match self {
                Self::Key(key) => key.get_public_key_async().await.map_err(SignerError::new),
                Self::Extension(extension) => extension.public_key().await,
            }
        }))
    }
}

impl AsyncSignEvent for WebSigner {
    type Error = SignerError;

    fn sign_event_async(&self, unsigned: UnsignedEvent) -> Signed<'_, Event> {
        Box::pin(SendWrapper::new(async move {
            match self {
                Self::Key(key) => key
                    .sign_event_async(unsigned)
                    .await
                    .map_err(SignerError::new),
                Self::Extension(extension) => extension.sign(unsigned).await,
            }
        }))
    }
}

impl AsyncNip44 for WebSigner {
    type Error = SignerError;

    fn nip44_encrypt_async<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> Signed<'a, String> {
        Box::pin(SendWrapper::new(async move {
            match self {
                Self::Key(key) => key
                    .nip44_encrypt_async(public_key, content)
                    .await
                    .map_err(SignerError::new),
                Self::Extension(extension) => {
                    let params = [public_key.to_hex().into(), content.into()];
                    extension.text("nip44.encrypt", &params).await
                }
            }
        }))
    }

    fn nip44_decrypt_async<'a>(
        &'a self,
        public_key: &'a PublicKey,
        payload: &'a str,
    ) -> Signed<'a, String> {
        Box::pin(SendWrapper::new(async move {
            match self {
                Self::Key(key) => key
                    .nip44_decrypt_async(public_key, payload)
                    .await
                    .map_err(SignerError::new),
                Self::Extension(extension) => {
                    let params = [public_key.to_hex().into(), payload.into()];
                    extension.text("nip44.decrypt", &params).await
                }
            }
        }))
    }
}
