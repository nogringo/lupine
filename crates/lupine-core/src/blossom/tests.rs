use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use nostr::event::{Event, Kind};
use nostr::key::{Keys, PublicKey};
use nostr::types::Timestamp;

use super::auth::{self, Verb};
use super::*;
use crate::content::{FileContent, FileKey, TransferOptions, download, upload};
use crate::crypto::random_bytes;
use crate::io::MemoryFile;

#[derive(Clone, Copy)]
struct Behavior {
    /// BUD-13 `PUT /<sha256>`.
    put: bool,
    /// BUD-13 `POST /<sha256>`.
    mirror: bool,
    ignores_ranges: bool,
    /// Answers ranges without `Content-Range`, as a browser shows them when the server does not expose it.
    hides_content_range: bool,
    /// Answers a range one byte further than asked.
    shifts_ranges: bool,
    get_needs_auth: bool,
    corrupts: bool,
    /// BUD-06 `HEAD /upload`.
    preflight: bool,
    /// Refuses what is not an image, as media hosts do.
    media_only: bool,
}

const MODERN: Behavior = Behavior {
    put: true,
    mirror: true,
    ignores_ranges: false,
    hides_content_range: false,
    shifts_ranges: false,
    get_needs_auth: false,
    corrupts: false,
    preflight: true,
    media_only: false,
};

/// Only BUD-02 `PUT /upload`, like most servers in November 2026.
const LEGACY: Behavior = Behavior {
    put: false,
    mirror: false,
    ignores_ranges: true,
    preflight: false,
    ..MODERN
};

struct Fake {
    behavior: Behavior,
    drive: PublicKey,
    origin: String,
    blobs: Mutex<HashMap<Sha256Hash, Bytes>>,
    /// `"<method> <path> <status>"`, with `<sha256>` standing for any hash.
    log: Mutex<Vec<String>>,
    failures: AtomicU32,
}

impl Fake {
    async fn start(behavior: Behavior, drive: PublicKey) -> Arc<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Arc::new(Self {
            behavior,
            drive,
            origin: format!("http://{}", listener.local_addr().unwrap()),
            blobs: Mutex::default(),
            log: Mutex::default(),
            failures: AtomicU32::new(0),
        });
        let app = Router::new().fallback(handle).with_state(Arc::clone(&fake));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fake
    }

    fn insert(&self, blob: &[u8]) -> Sha256Hash {
        let hash = Sha256Hash::digest(blob);
        self.blobs().insert(hash, Bytes::copy_from_slice(blob));
        hash
    }

    fn has(&self, hash: &Sha256Hash) -> bool {
        self.blobs().contains_key(hash)
    }

    fn remove(&self, hash: &Sha256Hash) {
        self.blobs().remove(hash);
    }

    fn count(&self, prefix: &str) -> usize {
        let log = self.log.lock().unwrap();
        log.iter().filter(|line| line.starts_with(prefix)).count()
    }

    fn fail_next(&self, requests: u32) {
        self.failures.store(requests, Ordering::SeqCst);
    }

    fn blobs(&self) -> std::sync::MutexGuard<'_, HashMap<Sha256Hash, Bytes>> {
        self.blobs.lock().unwrap()
    }

    /// BUD-11 validation, plus the drive key and the `server` tag.
    fn authorized(&self, headers: &HeaderMap, verb: &str, hash: Option<Sha256Hash>) -> bool {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Nostr "));
        let Some(json) = token.and_then(|t| URL_SAFE_NO_PAD.decode(t).ok()) else {
            return false;
        };
        let Ok(event) = Event::from_json(json) else {
            return false;
        };
        let now = Timestamp::now();
        let values = |name: &'static str| {
            event
                .tags
                .iter()
                .filter_map(move |tag| match tag.as_slice() {
                    [key, value, ..] if key == name => Some(value.as_str()),
                    _ => None,
                })
        };
        event.verify().is_ok()
            && event.kind == Kind::BlossomAuth
            && event.pubkey == self.drive
            && event.created_at <= now
            && values("expiration").any(|e| e.parse().is_ok_and(|e: u64| e > now.as_secs()))
            && values("t").any(|t| t == verb)
            && values("server").any(|s| s == "127.0.0.1")
            && hash.is_none_or(|h| values("x").any(|x| x == h.to_hex()))
    }

    fn descriptor(&self, hash: Sha256Hash, size: usize) -> Response {
        let descriptor = serde_json::json!({
            "url": format!("{}/{hash}.bin", self.origin),
            "sha256": hash,
            "size": size,
            "type": "application/octet-stream",
            "uploaded": Timestamp::now().as_secs(),
        });
        (StatusCode::CREATED, descriptor.to_string()).into_response()
    }

    fn store(&self, headers: &HeaderMap, path: Option<Sha256Hash>, body: Bytes) -> Response {
        let hash = Sha256Hash::digest(&body);
        if !self.authorized(headers, "upload", Some(path.unwrap_or(hash))) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if path.is_some_and(|path| path != hash) {
            return StatusCode::CONFLICT.into_response();
        }
        let size = body.len();
        self.blobs().insert(hash, body);
        self.descriptor(hash, size)
    }

    async fn mirror(&self, headers: &HeaderMap, hash: Sha256Hash, body: Bytes) -> Response {
        if !self.authorized(headers, "upload", Some(hash)) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let sources = String::from_utf8_lossy(&body).into_owned();
        for source in sources.lines().map(str::trim).filter(|s| !s.is_empty()) {
            let Ok(response) = reqwest::get(source).await else {
                continue;
            };
            let Ok(blob) = response.bytes().await else {
                continue;
            };
            if Sha256Hash::digest(&blob) == hash {
                let size = blob.len();
                self.blobs().insert(hash, blob);
                return self.descriptor(hash, size);
            }
        }
        StatusCode::BAD_GATEWAY.into_response()
    }

    fn serve(&self, method: &Method, headers: &HeaderMap, hash: Sha256Hash) -> Response {
        if self.behavior.get_needs_auth && !self.authorized(headers, "get", None) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let Some(blob) = self.blobs().get(&hash).cloned() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mut blob = blob.to_vec();
        if self.behavior.corrupts {
            blob[0] ^= 1;
        }
        if method == Method::HEAD {
            return StatusCode::OK.into_response();
        }
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok()?.strip_prefix("bytes=")?.split_once('-'))
            .and_then(|(start, end)| {
                Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()?))
            });
        match range {
            Some((start, end)) if !self.behavior.ignores_ranges => {
                let shift = usize::from(self.behavior.shifts_ranges);
                let (start, end) = (start + shift, end + shift);
                if end >= blob.len() || start > end {
                    return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                }
                let part = blob[start..=end].to_vec();
                if self.behavior.hides_content_range {
                    return (StatusCode::PARTIAL_CONTENT, part).into_response();
                }
                let content_range = format!("bytes {start}-{end}/{}", blob.len());
                let headers = [(header::CONTENT_RANGE, content_range)];
                (StatusCode::PARTIAL_CONTENT, headers, part).into_response()
            }
            _ => blob.into_response(),
        }
    }

    fn preflight(&self, headers: &HeaderMap) -> Response {
        if !self.authorized(headers, "upload", None) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let kind = headers.get("X-Content-Type").and_then(|v| v.to_str().ok());
        if self.behavior.media_only && !kind.is_some_and(|k| k.starts_with("image/")) {
            let reason = [("X-Reason", "only media files are accepted")];
            return (StatusCode::UNSUPPORTED_MEDIA_TYPE, reason).into_response();
        }
        StatusCode::OK.into_response()
    }

    fn options(&self, path: &str) -> Response {
        let methods = if path == "upload" {
            "HEAD, PUT, OPTIONS".to_owned()
        } else {
            let mut methods = vec!["GET", "HEAD", "DELETE", "OPTIONS"];
            if self.behavior.put {
                methods.push("PUT");
            }
            if self.behavior.mirror {
                methods.push("POST");
            }
            methods.join(", ")
        };
        let headers = [
            (header::ALLOW, methods.clone()),
            (header::ACCESS_CONTROL_ALLOW_METHODS, methods),
        ];
        (StatusCode::NO_CONTENT, headers).into_response()
    }
}

async fn handle(
    State(fake): State<Arc<Fake>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    let hash = path.parse::<Sha256Hash>().ok();
    let failing = fake
        .failures
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    let response = match (&method, path, hash) {
        _ if failing => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        (&Method::OPTIONS, path, _) => fake.options(path),
        (&Method::PUT, "upload", _) => fake.store(&headers, None, body),
        (&Method::HEAD, "upload", _) if fake.behavior.preflight => fake.preflight(&headers),
        (_, _, None) => StatusCode::NOT_FOUND.into_response(),
        (&Method::GET | &Method::HEAD, _, Some(hash)) => fake.serve(&method, &headers, hash),
        (&Method::PUT, _, Some(hash)) if fake.behavior.put => {
            fake.store(&headers, Some(hash), body)
        }
        (&Method::POST, _, Some(hash)) if fake.behavior.mirror => {
            fake.mirror(&headers, hash, body).await
        }
        (&Method::DELETE, _, Some(hash)) => {
            if !fake.authorized(&headers, "delete", Some(hash)) {
                StatusCode::UNAUTHORIZED.into_response()
            } else if fake.blobs().remove(&hash).is_some() {
                StatusCode::NO_CONTENT.into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };
    let shown = if hash.is_some() { "<sha256>" } else { path };
    let line = format!("{method} /{shown} {}", response.status().as_u16());
    fake.log.lock().unwrap().push(line);
    response
}

async fn setup(behaviors: &[Behavior]) -> (Vec<Arc<Fake>>, BlossomStore) {
    let keys = Keys::generate();
    let mut fakes = Vec::new();
    for &behavior in behaviors {
        fakes.push(Fake::start(behavior, keys.public_key()).await);
    }
    let store = BlossomStore::new(fakes.iter().map(|f| f.origin.clone()), &keys).unwrap();
    (fakes, store)
}

fn random_blob() -> Bytes {
    Bytes::copy_from_slice(&random_bytes::<1024>())
}

const OPTIONS: TransferOptions = TransferOptions {
    chunk_size: 1 << 20,
    concurrency: 3,
};

async fn upload_file(store: &BlossomStore, key: &FileKey, data: &[u8]) -> FileContent {
    let source = Arc::new(MemoryFile::new(data.to_vec()));
    upload(store, source, key, None, OPTIONS, &|_| {})
        .await
        .unwrap()
}

/// Downloads `range` and compares what was written with `data`.
async fn check_read(
    store: &BlossomStore,
    content: &FileContent,
    key: &FileKey,
    data: &[u8],
    range: std::ops::Range<u64>,
) {
    let sink = Arc::new(MemoryFile::default());
    let written = download(
        store,
        content,
        key,
        range.clone(),
        Arc::clone(&sink),
        OPTIONS,
        &|_| {},
    )
    .await
    .unwrap();
    assert!(written.start <= range.start && written.end >= range.end);
    let written = written.start as usize..written.end as usize;
    assert_eq!(sink.snapshot()[written.clone()], data[written]);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_round_trip_through_every_server() {
    let (fakes, store) = setup(&[MODERN, MODERN, LEGACY]).await;
    let data: Vec<u8> = (0..(5 << 19)).map(|i| (i % 251) as u8).collect();
    let key = FileKey::generate();
    let content = upload_file(&store, &key, &data).await;
    let chunks = content.chunks().len();
    assert_eq!(chunks, 3);

    for chunk in content.chunks() {
        assert!(fakes.iter().all(|f| f.has(&chunk.blob)));
    }
    assert_eq!(fakes[0].count("PUT /<sha256> 201"), chunks);
    assert_eq!(fakes[1].count("POST /<sha256> 201"), chunks, "mirrored");
    assert_eq!(fakes[1].count("PUT"), 0, "uploaded once");
    assert_eq!(fakes[2].count("PUT /upload 201"), chunks, "BUD-02 fallback");

    check_read(&store, &content, &key, &data, 0..data.len() as u64).await;
    let part = 100_000..1_500_000;
    check_read(&store, &content, &key, &data, part.clone()).await;
    assert!(fakes[0].count("GET /<sha256> 206") > 0);

    for chunk in content.chunks() {
        fakes[0].remove(&chunk.blob);
        fakes[1].remove(&chunk.blob);
    }
    check_read(&store, &content, &key, &data, part).await;
    assert!(fakes[2].count("GET /<sha256> 200") > 0, "ignores Range");
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_skip_servers_that_lost_or_corrupt_a_blob() {
    let corrupting = Behavior {
        corrupts: true,
        ..MODERN
    };
    let (fakes, store) = setup(&[corrupting, MODERN, MODERN]).await;
    let blob = random_blob();
    let hash = fakes[0].insert(&blob);
    fakes[2].insert(&blob);

    assert_eq!(store.get(hash).await.unwrap(), blob);
    for fake in &fakes {
        assert_eq!(fake.count("GET"), 1);
    }
    assert_eq!(
        store.get_range(hash, 10..20).await.unwrap(),
        blob.slice(10..20)
    );

    let unknown = Sha256Hash::digest(b"unknown");
    assert!(matches!(
        store.get(unknown).await,
        Err(Error::BlobNotFound(_))
    ));
    fakes[2].remove(&hash);
    assert!(matches!(
        store.get(hash).await,
        Err(Error::BlobHashMismatch(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn ranges_need_no_visible_content_range() {
    let hiding = Behavior {
        hides_content_range: true,
        ..MODERN
    };
    let (fakes, store) = setup(&[hiding]).await;
    let data: Vec<u8> = (0..(3 << 19)).map(|i| (i % 251) as u8).collect();
    let key = FileKey::generate();
    let content = upload_file(&store, &key, &data).await;
    check_read(&store, &content, &key, &data, 100_000..1_200_000).await;
    assert!(fakes[0].count("GET /<sha256> 206") > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn another_range_fails_to_decrypt() {
    let (_, store) = setup(&[MODERN]).await;
    let data: Vec<u8> = (0..(3 << 19)).map(|i| (i % 251) as u8).collect();
    let key = FileKey::generate();
    let content = upload_file(&store, &key, &data).await;
    let origins = setup(&[Behavior {
        hides_content_range: true,
        shifts_ranges: true,
        ..MODERN
    }])
    .await;
    for chunk in content.chunks() {
        origins.0[0].insert(&store.get(chunk.blob).await.unwrap());
    }
    let sink = Arc::new(MemoryFile::default());
    let read = download(
        &origins.1,
        &content,
        &key,
        100_000..200_000,
        sink,
        OPTIONS,
        &|_| {},
    )
    .await;
    assert!(matches!(read, Err(Error::Decrypt)), "{read:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn replicate_restores_lost_copies() {
    let (fakes, store) = setup(&[MODERN, MODERN, LEGACY]).await;
    let hash = fakes[1].insert(&random_blob());

    store.replicate(hash).await.unwrap();
    assert!(fakes.iter().all(|f| f.has(&hash)));
    assert_eq!(fakes[0].count("POST /<sha256> 201"), 1);
    assert_eq!(fakes[2].count("PUT /upload 201"), 1);

    store.replicate(hash).await.unwrap();
    assert_eq!(fakes[0].count("POST"), 1, "nothing to copy");

    let lost = Sha256Hash::digest(b"lost everywhere");
    assert!(matches!(
        store.replicate(lost).await,
        Err(Error::BlobNotFound(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_from_every_server() {
    let (fakes, store) = setup(&[MODERN, LEGACY]).await;
    let blob = random_blob();
    let hash = Sha256Hash::digest(&blob);
    store.put(hash, blob).await.unwrap();
    assert!(fakes.iter().all(|f| f.has(&hash)));

    store.delete(hash).await.unwrap();
    assert!(fakes.iter().all(|f| !f.has(&hash)));
    store.delete(hash).await.unwrap();
    assert!(fakes.iter().all(|f| f.count("DELETE /<sha256> 404") == 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_are_authorized_once_the_server_asks() {
    let private = Behavior {
        get_needs_auth: true,
        ..MODERN
    };
    let (fakes, store) = setup(&[private]).await;
    let blob = random_blob();
    let hash = fakes[0].insert(&blob);

    assert_eq!(store.get(hash).await.unwrap(), blob);
    assert_eq!(store.get(hash).await.unwrap(), blob);
    assert_eq!(fakes[0].count("GET /<sha256> 401"), 1);
    assert_eq!(fakes[0].count("GET /<sha256> 200"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_failures_are_retried() {
    let (fakes, store) = setup(&[MODERN]).await;
    let blob = random_blob();
    let hash = Sha256Hash::digest(&blob);

    fakes[0].fail_next(ATTEMPTS - 1);
    store.put(hash, blob.clone()).await.unwrap();
    assert!(fakes[0].has(&hash));

    fakes[0].fail_next(ATTEMPTS);
    let error = store.delete(hash).await.unwrap_err();
    assert!(matches!(error, Error::Http { status: 503, .. }), "{error}");
    assert!(error.is_transient());
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_servers_fail_transiently() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let store = BlossomStore::new([url], &Keys::generate()).unwrap();
    let blob = random_blob();

    let error = store
        .put(Sha256Hash::digest(&blob), blob)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Network { .. }), "{error}");
    assert!(error.is_transient());
}

#[test]
fn tokens_follow_bud_11() {
    let keys = Keys::generate();
    let hash = Sha256Hash::digest(b"blob");
    let header = auth::header(&keys, Verb::Delete, "cdn.example.com", &hash).unwrap();
    let token = header.strip_prefix("Nostr ").unwrap();
    assert!(
        !token.contains(['=', '+', '/']),
        "base64url without padding"
    );

    let event = Event::from_json(URL_SAFE_NO_PAD.decode(token).unwrap()).unwrap();
    event.verify().unwrap();
    assert_eq!(event.kind, Kind::BlossomAuth);
    assert_eq!(event.pubkey, keys.public_key());
    assert!(event.created_at <= Timestamp::now());
    let tags: Vec<_> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
    assert!(tags.contains(&vec!["t".into(), "delete".into()]));
    assert!(tags.contains(&vec!["server".into(), "cdn.example.com".into()]));
    assert!(tags.contains(&vec!["x".into(), hash.to_hex()]));
    let expiration = event.tags.expiration().unwrap();
    assert!(expiration > Timestamp::now());
}

#[test]
fn server_urls_keep_only_the_origin() {
    let keys = Keys::generate();
    let hash = Sha256Hash::digest(b"");
    let server = BlossomServer::new("https://CDN.Example.com/some/path/?q=1#f", keys.clone());
    let server = server.unwrap();
    assert_eq!(server.url(), "https://cdn.example.com");
    assert_eq!(
        server.blob_url(&hash).as_str(),
        format!("https://cdn.example.com/{hash}")
    );
    let local = BlossomServer::new("http://user:pw@127.0.0.1:3000", keys.clone()).unwrap();
    assert_eq!(local.url(), "http://127.0.0.1:3000");

    for invalid in ["ftp://cdn.example.com", "cdn.example.com", ""] {
        assert!(
            BlossomServer::new(invalid, keys.clone()).is_err(),
            "{invalid}"
        );
    }
    assert!(BlossomStore::new(Vec::<String>::new(), &keys).is_err());
}

/// LUPINE_BLOSSOM_SERVERS=https://a.example,https://b.example \
///   cargo test -p lupine-core live_servers -- --ignored
///
/// Uploads a small file with a throwaway key, reads it back, deletes it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs LUPINE_BLOSSOM_SERVERS, and uploads to them"]
async fn live_servers() {
    let urls = std::env::var("LUPINE_BLOSSOM_SERVERS").expect("LUPINE_BLOSSOM_SERVERS");
    let store = BlossomStore::new(urls.split(','), &Keys::generate()).unwrap();
    let mut data = vec![0; 5 << 19];
    getrandom::fill(&mut data).unwrap();
    let key = FileKey::generate();
    let content = upload_file(&store, &key, &data).await;

    for chunk in content.chunks() {
        store.replicate(chunk.blob).await.unwrap();
    }
    check_read(&store, &content, &key, &data, 0..data.len() as u64).await;
    check_read(&store, &content, &key, &data, 100_000..1_500_000).await;

    for chunk in content.chunks() {
        store.delete(chunk.blob).await.unwrap();
        for server in store.servers() {
            let kept = server.has(chunk.blob).await.unwrap();
            assert!(!kept, "{} kept {}", server.url(), chunk.blob);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_checks_spot_media_hosts() {
    let media = Behavior {
        media_only: true,
        ..MODERN
    };
    let (fakes, store) = setup(&[MODERN, LEGACY, media]).await;
    let servers = store.servers();
    servers[0].check_upload(8 << 20).await.unwrap();
    servers[1].check_upload(8 << 20).await.unwrap();
    let refused = servers[2].check_upload(8 << 20).await.unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::Error::Http {
                status: 415,
                reason: Some(_),
                ..
            }
        ),
        "{refused}"
    );
    assert_eq!(fakes[1].count("HEAD /upload 404"), 1, "no BUD-06");

    let unknown_key = BlossomServer::new(&fakes[0].origin, Keys::generate()).unwrap();
    unknown_key.check_upload(8 << 20).await.unwrap();
}
