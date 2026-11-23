//! Positional I/O, so chunks are read and written concurrently and in any order.

use std::io;
use std::sync::Mutex;

pub trait ReadAt: Send + Sync {
    fn size(&self) -> io::Result<u64>;
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

pub trait WriteAt: Send + Sync {
    fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
}

#[cfg(any(unix, windows))]
impl ReadAt for std::fs::File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    #[cfg(unix)]
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match self.seek_read(buf, offset)? {
                0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                n => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
            }
        }
        Ok(())
    }
}

#[cfg(any(unix, windows))]
impl WriteAt for std::fs::File {
    #[cfg(unix)]
    fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::write_all_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn write_all_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match self.seek_write(buf, offset)? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                n => {
                    buf = &buf[n..];
                    offset += n as u64;
                }
            }
        }
        Ok(())
    }
}

/// In-memory file, for tests and for platforms without a filesystem.
#[derive(Default)]
pub struct MemoryFile(Mutex<Vec<u8>>);

impl MemoryFile {
    pub fn new(data: Vec<u8>) -> Self {
        Self(Mutex::new(data))
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0.into_inner().unwrap_or_else(|e| e.into_inner())
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ReadAt for MemoryFile {
    fn size(&self) -> io::Result<u64> {
        Ok(self.lock().len() as u64)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let data = self.lock();
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let end = start
            .checked_add(buf.len())
            .filter(|&end| end <= data.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(&data[start..end]);
        Ok(())
    }
}

impl WriteAt for MemoryFile {
    fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let mut data = self.lock();
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::InvalidInput)?;
        let end = start + buf.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(buf);
        Ok(())
    }
}
