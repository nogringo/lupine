use std::future::Future;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use nostr::key::Keys;
use reqwest::header::{self, HeaderMap, HeaderName};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use serde::Deserialize;
use tokio::sync::OnceCell;

use super::auth::{self, Verb};
use crate::blob::slice;
use crate::error::{Error, Result};
use crate::ids::Sha256Hash;

/// Chunks are at most 64 MiB, but the chunk table of a huge file is larger.
const MAX_BLOB_LEN: u64 = 1 << 30;
const MAX_DESCRIPTOR_LEN: u64 = 64 * 1024;
pub(super) const ATTEMPTS: u32 = 3;

/// One Blossom server, reached with the drive key.
pub struct BlossomServer {
    base: Url,
    domain: String,
    origin: String,
    keys: Keys,
    http: Client,
    features: OnceCell<Features>,
    get_needs_auth: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Features {
    /// BUD-13 `PUT /<sha256>`.
    put: bool,
    /// BUD-13 `POST /<sha256>`.
    mirror: bool,
}

#[derive(Deserialize)]
struct Descriptor {
    sha256: Sha256Hash,
    size: Option<u64>,
}

impl BlossomServer {
    /// Blossom serves every endpoint from the root, so only the origin of
    /// `url` is kept.
    pub fn new(url: &str, keys: Keys) -> Result<Self> {
        let invalid = || Error::Malformed("invalid Blossom server URL");
        let mut base = Url::parse(url).map_err(|_| invalid())?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(invalid());
        }
        let domain = base.host_str().ok_or_else(invalid)?.to_owned();
        let origin = base.origin().ascii_serialization();
        base = Url::parse(&origin).map_err(|_| invalid())?;
        let http = Client::builder()
            .user_agent(concat!("lupine/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| Error::Network {
                server: origin.clone(),
                message: describe(&e),
            })?;
        Ok(Self {
            base,
            domain,
            origin,
            keys,
            http,
            features: OnceCell::new(),
            get_needs_auth: AtomicBool::new(false),
        })
    }

    /// Such as `https://cdn.example.com`.
    pub fn url(&self) -> &str {
        &self.origin
    }

    pub fn blob_url(&self, hash: &Sha256Hash) -> Url {
        self.base
            .join(&hash.to_hex())
            .expect("a hash is a valid path")
    }

    /// BUD-01 `HEAD /<sha256>`.
    pub async fn has(&self, hash: Sha256Hash) -> Result<bool> {
        let response = self.fetch(Method::HEAD, hash, None).await?;
        match response.status() {
            status if status.is_success() => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            _ => Err(self.failure(&response)),
        }
    }

    /// BUD-01 `GET /<sha256>`. The blob is not checked against its hash.
    pub async fn get(&self, hash: Sha256Hash) -> Result<Bytes> {
        let response = self.fetch(Method::GET, hash, None).await?;
        match response.status() {
            status if status.is_success() => self.body(response, MAX_BLOB_LEN).await,
            StatusCode::NOT_FOUND => Err(Error::BlobNotFound(hash)),
            _ => Err(self.failure(&response)),
        }
    }

    /// BUD-01 `GET /<sha256>` with a `Range` header. When the server ignores
    /// it and sends the whole blob, only the range is kept.
    pub async fn get_range(&self, hash: Sha256Hash, range: Range<u64>) -> Result<Bytes> {
        let outside = || Error::Malformed("range outside the blob");
        if range.start > range.end {
            return Err(outside());
        }
        if range.is_empty() {
            return Ok(Bytes::new());
        }
        let response = self.fetch(Method::GET, hash, Some(&range)).await?;
        let len = range.end - range.start;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                if content_range_start(response.headers()) != Some(range.start) {
                    return Err(Error::Malformed("server sent another range"));
                }
                let part = self.body(response, len).await?;
                if part.len() as u64 != len {
                    return Err(outside());
                }
                Ok(part)
            }
            StatusCode::OK => slice(self.body(response, MAX_BLOB_LEN).await?, range),
            StatusCode::NOT_FOUND => Err(Error::BlobNotFound(hash)),
            StatusCode::RANGE_NOT_SATISFIABLE => Err(outside()),
            _ => Err(self.failure(&response)),
        }
    }

    /// Stores the blob with BUD-13 `PUT /<sha256>`, or BUD-02 `PUT /upload`
    /// on servers that do not advertise it. Transient failures are retried.
    pub async fn upload(&self, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        retrying(|| self.upload_once(hash, blob.clone())).await
    }

    async fn upload_once(&self, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        if self.features().await?.put {
            match self.put(self.blob_url(&hash), hash, blob.clone()).await {
                Err(Error::Http { status: 405, .. }) => {}
                result => return result,
            }
        }
        let legacy = self.base.join("upload").expect("a valid path");
        self.put(legacy, hash, blob).await
    }

    async fn put(&self, url: Url, hash: Sha256Hash, blob: Bytes) -> Result<()> {
        let len = blob.len() as u64;
        let request = self
            .http
            .put(url)
            .header(header::AUTHORIZATION, self.auth(Verb::Upload, &hash)?)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header("X-SHA-256", hash.to_hex())
            .body(blob);
        let response = self.send(request).await?;
        self.stored(response, hash, Some(len)).await
    }

    /// Whether the server can fetch blobs from other servers itself.
    pub async fn can_mirror(&self) -> bool {
        self.features().await.is_ok_and(|f| f.mirror)
    }

    /// BUD-13 `POST /<sha256>`: the server fetches the blob from any of
    /// `sources`, so the device does not upload it again.
    pub async fn mirror(&self, hash: Sha256Hash, sources: &[Url]) -> Result<()> {
        let list = sources.iter().map(Url::as_str).collect::<Vec<_>>();
        let request = self
            .http
            .post(self.blob_url(&hash))
            .header(header::AUTHORIZATION, self.auth(Verb::Upload, &hash)?)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(list.join("\n"));
        let response = self.send(request).await?;
        self.stored(response, hash, None).await
    }

    /// BUD-12 `DELETE /<sha256>`. Succeeds when the blob is already absent.
    /// Transient failures are retried.
    pub async fn delete(&self, hash: Sha256Hash) -> Result<()> {
        retrying(|| async move {
            let request = self
                .http
                .delete(self.blob_url(&hash))
                .header(header::AUTHORIZATION, self.auth(Verb::Delete, &hash)?);
            let response = self.send(request).await?;
            match response.status() {
                status if status.is_success() || status == StatusCode::NOT_FOUND => Ok(()),
                _ => Err(self.failure(&response)),
            }
        })
        .await
    }

    /// `GET` or `HEAD /<sha256>`, authorized once the server asks for it.
    async fn fetch(
        &self,
        method: Method,
        hash: Sha256Hash,
        range: Option<&Range<u64>>,
    ) -> Result<Response> {
        let mut authorize = self.get_needs_auth.load(Ordering::Relaxed);
        loop {
            let mut request = self.http.request(method.clone(), self.blob_url(&hash));
            if let Some(range) = range {
                let value = format!("bytes={}-{}", range.start, range.end - 1);
                request = request.header(header::RANGE, value);
            }
            if authorize {
                request = request.header(header::AUTHORIZATION, self.auth(Verb::Get, &hash)?);
            }
            let response = self.send(request).await?;
            if response.status() != StatusCode::UNAUTHORIZED || authorize {
                return Ok(response);
            }
            authorize = true;
            self.get_needs_auth.store(true, Ordering::Relaxed);
        }
    }

    /// BUD-01 feature discovery, once per server.
    async fn features(&self) -> Result<Features> {
        let features = self.features.get_or_try_init(|| async {
            let url = self.blob_url(&Sha256Hash::digest(b""));
            let response = self.send(self.http.request(Method::OPTIONS, url)).await?;
            let status = response.status();
            if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                return Err(self.failure(&response));
            }
            let headers = response.headers();
            let supports = |method| {
                status.is_success()
                    && allows(headers, header::ALLOW, method)
                    && allows(headers, header::ACCESS_CONTROL_ALLOW_METHODS, method)
            };
            Ok(Features {
                put: supports("PUT"),
                mirror: supports("POST"),
            })
        });
        features.await.copied()
    }

    /// Checks the blob descriptor that answers an upload or a mirror.
    async fn stored(&self, response: Response, hash: Sha256Hash, len: Option<u64>) -> Result<()> {
        if !response.status().is_success() {
            return Err(self.failure(&response));
        }
        let body = self.body(response, MAX_DESCRIPTOR_LEN).await?;
        let descriptor: Descriptor = serde_json::from_slice(&body)
            .map_err(|_| Error::Malformed("invalid blob descriptor"))?;
        let size_differs = len
            .zip(descriptor.size)
            .is_some_and(|(sent, stored)| sent != stored);
        if descriptor.sha256 != hash || size_differs {
            return Err(Error::BlobHashMismatch(hash));
        }
        Ok(())
    }

    async fn body(&self, mut response: Response, limit: u64) -> Result<Bytes> {
        let too_large = || Error::Malformed("server sent more than expected");
        let expected = response.content_length().unwrap_or(0);
        if expected > limit {
            return Err(too_large());
        }
        let mut body = BytesMut::with_capacity(expected as usize);
        while let Some(chunk) = response.chunk().await.map_err(|e| self.network(&e))? {
            if (body.len() + chunk.len()) as u64 > limit {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body.freeze())
    }

    async fn send(&self, request: RequestBuilder) -> Result<Response> {
        request.send().await.map_err(|e| self.network(&e))
    }

    fn auth(&self, verb: Verb, hash: &Sha256Hash) -> Result<String> {
        auth::header(&self.keys, verb, &self.domain, hash)
    }

    fn failure(&self, response: &Response) -> Error {
        let reason = response.headers().get("X-Reason");
        Error::Http {
            server: self.origin.clone(),
            status: response.status().as_u16(),
            reason: reason.and_then(|r| r.to_str().ok()).map(str::to_owned),
        }
    }

    fn network(&self, error: &reqwest::Error) -> Error {
        Error::Network {
            server: self.origin.clone(),
            message: describe(error),
        }
    }
}

pub(super) async fn retrying<T, F, Fut>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 1;
    loop {
        match op().await {
            Err(e) if e.is_transient() && attempt < ATTEMPTS => {
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
            result => return result,
        }
    }
}

pub(super) fn backoff(attempt: u32) -> Duration {
    let base = if cfg!(test) { 5 } else { 500 };
    Duration::from_millis(base) * 4u32.pow(attempt - 1)
}

/// BUD-01: a method is supported only when both lists advertise it.
fn allows(headers: &HeaderMap, name: HeaderName, method: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|m| m.trim().eq_ignore_ascii_case(method))
}

/// Start of `Content-Range: bytes <start>-<end>/<size>`.
fn content_range_start(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(header::CONTENT_RANGE)?.to_str().ok()?;
    let (start, _) = value.strip_prefix("bytes ")?.split_once('-')?;
    start.trim().parse().ok()
}

/// reqwest's own message rarely says what failed, its sources do.
fn describe(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}
