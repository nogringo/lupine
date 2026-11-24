//! File content the page hands over, and where downloads land. Workers read
//! blobs and write OPFS files synchronously, as the core's positional I/O
//! expects.

use std::io;

use js_sys::Uint8Array;
use lupine_core::io::{ReadAt, WriteAt};
use web_sys::{Blob, FileReaderSync, FileSystemReadWriteOptions, FileSystemSyncAccessHandle};

use crate::error::describe;

/// A `File` from a file picker or a drop.
pub(crate) struct BlobSource {
    blob: Blob,
    reader: FileReaderSync,
}

impl BlobSource {
    pub(crate) fn new(blob: Blob) -> io::Result<Self> {
        let reader = FileReaderSync::new().map_err(js_io)?;
        Ok(Self { blob, reader })
    }
}

// SAFETY: the crate refuses to build with threads.
unsafe impl Send for BlobSource {}
unsafe impl Sync for BlobSource {}

impl ReadAt for BlobSource {
    fn size(&self) -> io::Result<u64> {
        Ok(self.blob.size() as u64)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let end = offset + buf.len() as u64;
        if end > self.size()? {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let part = self
            .blob
            .slice_with_f64_and_f64(offset as f64, end as f64)
            .map_err(js_io)?;
        // Fails, rather than mix contents, when the file changed on disk.
        let buffer = self.reader.read_as_array_buffer(&part).map_err(js_io)?;
        let data = Uint8Array::new(&buffer);
        if data.length() as usize != buf.len() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        data.copy_to(buf);
        Ok(())
    }
}

/// A file in OPFS, opened for a download.
pub(crate) struct HandleSink(FileSystemSyncAccessHandle);

impl HandleSink {
    pub(crate) fn new(handle: FileSystemSyncAccessHandle) -> Self {
        Self(handle)
    }
}

// SAFETY: the crate refuses to build with threads.
unsafe impl Send for HandleSink {}
unsafe impl Sync for HandleSink {}

impl WriteAt for HandleSink {
    fn write_all_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            let options = FileSystemReadWriteOptions::new();
            options.set_at(offset as f64);
            let written = self
                .0
                .write_with_u8_array_and_options(buf, &options)
                .map_err(js_io)? as usize;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            buf = &buf[written..];
            offset += written as u64;
        }
        Ok(())
    }
}

fn js_io(error: wasm_bindgen::JsValue) -> io::Error {
    io::Error::other(describe(&error))
}
