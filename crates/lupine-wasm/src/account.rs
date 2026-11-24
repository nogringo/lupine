//! Signing in: the user's key opens a drive or creates one, then this
//! device keeps only the drive's keys.

use std::sync::{Arc, Mutex};

use js_sys::Function;
use lupine_core::account::{Account, Discovery};
use lupine_core::descriptor::FoundDrive;
use lupine_core::identity::{NOSTR_CONNECT_RELAYS, OpenUrl, UserKey};
use lupine_core::store::{DescriptorRecord, DriveRecord};
use nostr::types::RelayUrl;
use send_wrapper::SendWrapper;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::local::{ImportedDrive, npub, set_up};
use crate::reader::Cancellation;
use crate::signer::{Extension, WebSigner};
use crate::{CoreError, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface RemoteDrive {
    id: string;
    name: string;
}

export interface DriveLists {
    relays: string[];
    servers: string[];
}
"#;

#[derive(Serialize)]
struct RemoteDrive {
    id: String,
    name: String,
}

#[derive(Serialize)]
pub(crate) struct DriveLists {
    pub(crate) relays: Vec<String>,
    pub(crate) servers: Vec<String>,
}

/// `open_url(url)` shows the page where a remote signer asks the user to
/// approve Lupine. Browsers block popups no click opened, so the page offers
/// a link.
fn opener(open_url: Function) -> OpenUrl {
    let open_url = SendWrapper::new(open_url);
    Arc::new(move |url| {
        let _ = open_url.call1(&JsValue::NULL, &url.as_str().into());
    })
}

/// The user, signed in until [`close`](Self::close).
#[wasm_bindgen]
pub struct SignIn {
    account: Account<WebSigner>,
    found: tokio::sync::Mutex<Vec<FoundDrive>>,
}

/// Signs in with an nsec or a `bunker://` link.
#[wasm_bindgen(js_name = signIn)]
pub async fn sign_in(key: String, open_url: Function) -> Result<SignIn, CoreError> {
    let key = UserKey::parse(&key, Some(opener(open_url)))?;
    SignIn::new(WebSigner::Key(key)).await
}

/// Signs in with a NIP-07 extension. `call(method, params)` runs
/// `window.nostr[method](...params)` in the page, `nip44.encrypt` and
/// `nip44.decrypt` included.
#[wasm_bindgen(js_name = signInWithExtension)]
pub async fn sign_in_with_extension(
    #[wasm_bindgen(
        unchecked_param_type = "(method: string, params: unknown[]) => Promise<unknown>"
    )]
    call: Function,
) -> Result<SignIn, CoreError> {
    SignIn::new(WebSigner::Extension(Extension::new(call))).await
}

/// A `nostrconnect://` link for a signer app to scan or open.
#[wasm_bindgen]
pub struct SignerPairing {
    link: String,
    key: Mutex<Option<UserKey>>,
    cancel: Cancellation,
}

#[wasm_bindgen(js_name = pairSigner)]
pub fn pair_signer(open_url: Function) -> Result<SignerPairing, CoreError> {
    let relays = NOSTR_CONNECT_RELAYS
        .iter()
        .map(|url| RelayUrl::parse(url).expect("a valid relay URL"))
        .collect();
    let (link, key) = UserKey::pair(relays, Some(opener(open_url)))?;
    Ok(SignerPairing {
        link,
        key: Mutex::new(Some(key)),
        cancel: Cancellation::default(),
    })
}

#[wasm_bindgen]
impl SignerPairing {
    #[wasm_bindgen(getter)]
    pub fn link(&self) -> String {
        self.link.clone()
    }

    /// Waits for the user to accept in the signer, then signs in. Once
    /// only, until [`cancel`](Self::cancel).
    pub async fn accepted(&self) -> Result<SignIn, CoreError> {
        let key = self
            .key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or(CoreError::Cancelled)?;
        let accepted = tokio::select! {
            () = self.cancel.cancelled() => Err(CoreError::Cancelled),
            accepted = key.public_key() => accepted.map_err(CoreError::from),
        };
        if let Err(e) = accepted {
            key.shutdown().await;
            return Err(e);
        }
        SignIn::new(WebSigner::Key(key)).await
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl SignIn {
    async fn new(signer: WebSigner) -> Result<Self, CoreError> {
        let signer = Arc::new(signer);
        match Account::sign_in(Arc::clone(&signer), &Discovery::default()).await {
            Ok(account) => Ok(Self {
                account,
                found: tokio::sync::Mutex::default(),
            }),
            Err(e) => {
                signer.shutdown().await;
                Err(e.into())
            }
        }
    }

    fn set_up(
        &self,
        path: &str,
        found: &FoundDrive,
        lists: lupine_core::account::DriveLists,
    ) -> Result<JsValue, CoreError> {
        let descriptor = &found.descriptor;
        let record = DriveRecord {
            id: descriptor.id,
            pubkey: descriptor.keys().public_key(),
            epoch: descriptor.epoch,
            relays: lists.relays,
            servers: lists.servers,
        };
        let kept = DescriptorRecord {
            event: found.event.clone(),
            name: descriptor.name.clone(),
            user_relays: self.account.relays().urls().to_vec(),
        };
        set_up(path, &record, Some(&kept))?;
        to_js(&ImportedDrive {
            id: descriptor.id.to_hex(),
            secret: descriptor.secret.to_secret_hex(),
            recovery_key: descriptor.recovery_key(&record.relays).encode(),
        })
    }
}

#[wasm_bindgen]
impl SignIn {
    /// The user's npub.
    #[wasm_bindgen(getter)]
    pub fn user(&self) -> String {
        npub(&self.account.public_key())
    }

    #[wasm_bindgen(getter, js_name = usesRemoteSigner)]
    pub fn uses_remote_signer(&self) -> bool {
        self.account.signer().is_remote()
    }

    /// The user's drives, by name.
    #[wasm_bindgen(unchecked_return_type = "RemoteDrive[]")]
    pub async fn drives(&self) -> Result<JsValue, CoreError> {
        let found = self.account.drives().await?;
        let drives: Vec<_> = found
            .iter()
            .map(|drive| RemoteDrive {
                id: drive.descriptor.id.to_hex(),
                name: drive.descriptor.name.clone(),
            })
            .collect();
        *self.found.lock().await = found;
        to_js(&drives)
    }

    /// The user's own inbox relays and Blossom servers, to suggest for a
    /// new drive. Empty when the user lists none.
    #[wasm_bindgen(js_name = suggestedLists, unchecked_return_type = "DriveLists")]
    pub async fn suggested_lists(&self) -> Result<JsValue, CoreError> {
        let lists = self.account.suggestions().await.unwrap_or_default();
        to_js(&DriveLists {
            relays: lists.relays.iter().map(ToString::to_string).collect(),
            servers: lists.servers,
        })
    }

    /// Sets up the database at `path` for drive `id` of [`drives`](Self::drives).
    #[wasm_bindgen(js_name = openDrive, unchecked_return_type = "ImportedDrive")]
    pub async fn open_drive(&self, path: String, id: String) -> Result<JsValue, CoreError> {
        let found = self
            .found
            .lock()
            .await
            .iter()
            .find(|drive| drive.descriptor.id.to_hex() == id)
            .cloned()
            .ok_or(CoreError::NoSuchItem)?;
        let lists = self.account.open(&found).await?;
        self.set_up(&path, &found, lists)
    }

    /// Creates a drive and sets up the database at `path` for it. Relay and
    /// server addresses may leave out their scheme.
    #[wasm_bindgen(js_name = createDrive, unchecked_return_type = "ImportedDrive")]
    pub async fn create_drive(
        &self,
        path: String,
        name: String,
        relays: Vec<String>,
        servers: Vec<String>,
    ) -> Result<JsValue, CoreError> {
        let (relays, servers) = (relay_urls(&relays)?, server_urls(&servers));
        let name = match name.trim() {
            "" => "Lupine",
            name => name,
        };
        let (found, lists) = self.account.create_drive(name, &relays, &servers).await?;
        self.set_up(&path, &found, lists)
    }

    pub async fn close(&self) {
        self.account.shutdown().await;
        self.account.signer().shutdown().await;
    }
}

/// Addresses typed one per line, which may leave out their scheme.
pub(crate) fn relay_urls(relays: &[String]) -> Result<Vec<RelayUrl>, CoreError> {
    relays
        .iter()
        .filter_map(|url| with_scheme(url, "wss://"))
        .map(|url| {
            RelayUrl::parse(&url).map_err(|_| CoreError::Check(format!("invalid relay: {url}")))
        })
        .collect()
}

pub(crate) fn server_urls(servers: &[String]) -> Vec<String> {
    servers
        .iter()
        .filter_map(|url| with_scheme(url, "https://"))
        .collect()
}

fn with_scheme(url: &str, scheme: &str) -> Option<String> {
    match url.trim() {
        "" => None,
        url if url.contains("://") => Some(url.to_owned()),
        url => Some(format!("{scheme}{url}")),
    }
}
