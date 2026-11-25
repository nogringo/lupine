//! Share links: the drive's links, and opening one, which takes no drive,
//! no database and no key but the link's.

use std::io;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use js_sys::{Function, Uint8Array};
use lupine_core::Error;
use lupine_core::blossom::BlossomStore;
use lupine_core::content::Progress;
use lupine_core::fetch::FileReader;
use lupine_core::io::WriteAt;
use lupine_core::share::{self, LinkState, Share, ShareLink, Shown};
use nostr::key::Keys;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::CoreError;
use crate::reader::progress;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface ShareInfo {
    id: string;
    label: string;
    /** Unix milliseconds. */
    created?: number;
    /** Unix milliseconds, absent for a link that does not expire. */
    expires?: number;
    /** The fragment of the link's URL. */
    link: string;
}
"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareInfo {
    id: String,
    label: String,
    created: Option<i64>,
    expires: Option<u64>,
    link: String,
}

impl From<Share> for ShareInfo {
    fn from(share: Share) -> Self {
        Self {
            id: share.node.to_hex(),
            label: share.label,
            created: share.created,
            expires: share.meta.expires.map(|secs| secs * 1000),
            link: share.meta.link().encode(),
        }
    }
}

#[wasm_bindgen]
pub struct SharedFile {
    shown: Shown,
    reader: FileReader<BlossomStore>,
}

/// What the link in the fragment shows, read from its relays.
#[wasm_bindgen(js_name = openShareLink)]
pub async fn open_share_link(fragment: String) -> Result<SharedFile, CoreError> {
    let Ok(link) = ShareLink::decode(&fragment) else {
        return Err(CoreError::Check(
            "this link is incomplete: ask whoever sent it for the whole link".into(),
        ));
    };
    match share::open(&link).await? {
        LinkState::Live { shown, .. } => SharedFile::new(shown),
        LinkState::Revoked => Err(CoreError::Check("this file is no longer shared".into())),
        LinkState::Expired => Err(CoreError::Check("this link expired".into())),
        LinkState::Empty => Err(CoreError::Check("this link shows nothing".into())),
    }
}

/// The file that [`SharedFile::shown`] describes, without asking relays
/// again: how the page hands it to its service worker.
#[wasm_bindgen(js_name = openSharedFile)]
pub fn open_shared_file(shown: String) -> Result<SharedFile, CoreError> {
    let shown: Shown =
        serde_json::from_str(&shown).map_err(|_| CoreError::Check("invalid shared file".into()))?;
    SharedFile::new(shown)
}

#[wasm_bindgen]
impl SharedFile {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.shown.name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn size(&self) -> f64 {
        self.shown.file.content.size as f64
    }

    /// For [`open_shared_file`].
    #[wasm_bindgen(getter)]
    pub fn shown(&self) -> String {
        serde_json::to_string(&self.shown).expect("a shared file serializes")
    }

    /// The whole content, in memory.
    #[wasm_bindgen(unchecked_return_type = "Uint8Array<ArrayBuffer>")]
    pub async fn read(
        &self,
        #[wasm_bindgen(unchecked_param_type = "(bytes: number) => void")] observer: Function,
    ) -> Result<Uint8Array, CoreError> {
        let size = self.shown.file.content.size;
        self.read_range(0..size, &progress(&observer)).await
    }

    /// Bytes `start` to `end` (excluded) of the content.
    #[wasm_bindgen(js_name = readRange, unchecked_return_type = "Uint8Array<ArrayBuffer>")]
    pub async fn read_range_js(&self, start: f64, end: f64) -> Result<Uint8Array, CoreError> {
        let size = self.shown.file.content.size;
        if !(0.0 <= start && start <= end && end <= size as f64) {
            return Err(CoreError::Check("range outside the file".into()));
        }
        self.read_range(start as u64..end as u64, &|_| {}).await
    }
}

impl SharedFile {
    fn new(shown: Shown) -> Result<Self, CoreError> {
        // A server that asks link readers for authorization gets a token from a throwaway key.
        let blobs = BlossomStore::new(&shown.servers, &Keys::generate())?;
        Ok(Self {
            shown,
            reader: FileReader::new(blobs),
        })
    }

    async fn read_range(
        &self,
        range: Range<u64>,
        progress: Progress<'_>,
    ) -> Result<Uint8Array, CoreError> {
        let sink = Arc::new(RangeSink::new(range.clone()));
        self.reader
            .read_into(
                &self.shown.file,
                range.clone(),
                1,
                Arc::clone(&sink),
                progress,
            )
            .await
            .map_err(|e| match e {
                Error::BlobNotFound(_) => {
                    CoreError::Check("the file is no longer on its servers".into())
                }
                e => e.into(),
            })?;
        let data = sink.data.lock().unwrap_or_else(|e| e.into_inner());
        if data.len() as u64 != range.end - range.start {
            return Err(CoreError::Check("the read came back short".into()));
        }
        Ok(Uint8Array::from(data.as_slice()))
    }
}

/// Keeps the bytes of `range` that a read writes: reads write whole
/// segments, which may start before it and end after it.
struct RangeSink {
    range: Range<u64>,
    data: Mutex<Vec<u8>>,
}

impl RangeSink {
    fn new(range: Range<u64>) -> Self {
        Self {
            range,
            data: Mutex::new(Vec::new()),
        }
    }
}

impl WriteAt for RangeSink {
    fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let start = offset.max(self.range.start);
        let end = (offset + buf.len() as u64).min(self.range.end);
        if start >= end {
            return Ok(());
        }
        let at = (start - self.range.start) as usize;
        let len = (end - start) as usize;
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if data.len() < at + len {
            data.resize(at + len, 0);
        }
        let from = (start - offset) as usize;
        data[at..at + len].copy_from_slice(&buf[from..from + len]);
        Ok(())
    }
}
