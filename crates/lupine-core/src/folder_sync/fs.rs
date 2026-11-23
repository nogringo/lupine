//! File system operations of a pass, which never replace what they did not
//! expect to find.

use std::fs::{self, FileTimes};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::scan;
use super::{STAGING, Stat};
use crate::crypto::random_bytes;

pub(super) fn millis(nanos: i64) -> i64 {
    nanos.div_euclid(1_000_000)
}

/// Renames `from` to `to` unless something is at `to`, other than `from`
/// itself under a name differing only in case.
pub(super) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    if let Ok(existing) = fs::symlink_metadata(to) {
        let source = fs::symlink_metadata(from)?;
        if same_file(&existing, &source) {
            return fs::rename(from, to);
        }
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    exclusive_rename(from, to)
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

#[cfg(not(unix))]
fn same_file(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    false
}

#[cfg(target_vendor = "apple")]
fn exclusive_rename(from: &Path, to: &Path) -> io::Result<()> {
    renamex(from, to, libc::RENAME_EXCL)
}

#[cfg(target_vendor = "apple")]
fn renamex(from: &Path, to: &Path, flags: libc::c_uint) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both are NUL-terminated paths that outlive the call.
    let renamed = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) };
    if renamed == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Checked by the caller just before, which leaves a short race.
#[cfg(not(target_vendor = "apple"))]
fn exclusive_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Puts `new` at `target` in place of `seen`, the file the scan found
/// there. `false`, with nothing replaced, when another file is there now.
#[cfg(target_vendor = "apple")]
pub(super) fn replace(new: &Path, target: &Path, seen: Stat) -> io::Result<bool> {
    exchange(new, target)?;
    if scan::probe(new)? == Some((false, seen)) {
        let _ = fs::remove_file(new);
        return Ok(true);
    }
    exchange(new, target)?;
    Ok(false)
}

/// Checked just before, which leaves a short race.
#[cfg(not(target_vendor = "apple"))]
pub(super) fn replace(new: &Path, target: &Path, seen: Stat) -> io::Result<bool> {
    if scan::probe(target)? != Some((false, seen)) {
        return Ok(false);
    }
    fs::rename(new, target)?;
    Ok(true)
}

#[cfg(target_vendor = "apple")]
fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    renamex(a, b, libc::RENAME_SWAP)
}

/// Sets the modification time of the file at `path`, Unix milliseconds,
/// provided it is still `seen`. Returns what it is after, `None` when it
/// changed.
pub(super) fn touch(path: &Path, seen: Stat, modified: Option<i64>) -> io::Result<Option<Stat>> {
    let file = fs::File::options().write(true).open(path)?;
    let before = file.metadata()?;
    if before.is_dir() || scan::stat(&before) != seen {
        return Ok(None);
    }
    if let Some(modified) = modified {
        file.set_times(FileTimes::new().set_modified(time(modified)))?;
    }
    Ok(Some(scan::stat(&file.metadata()?)))
}

/// Unix milliseconds; `None` keeps the current time.
pub(super) fn set_times(
    path: &Path,
    modified: Option<i64>,
    created: Option<i64>,
) -> io::Result<()> {
    if modified.is_none() && created.is_none() {
        return Ok(());
    }
    let file = fs::File::options().write(true).open(path)?;
    let mut times = FileTimes::new();
    if let Some(modified) = modified {
        times = times.set_modified(time(modified));
    }
    file.set_times(with_created(times, created))
}

#[cfg(target_os = "macos")]
fn with_created(times: FileTimes, created: Option<i64>) -> FileTimes {
    use std::os::macos::fs::FileTimesExt;
    match created {
        Some(created) => times.set_created(time(created)),
        None => times,
    }
}

#[cfg(not(target_os = "macos"))]
fn with_created(times: FileTimes, _: Option<i64>) -> FileTimes {
    times
}

fn time(millis: i64) -> SystemTime {
    let offset = Duration::from_millis(millis.unsigned_abs());
    if millis >= 0 {
        UNIX_EPOCH + offset
    } else {
        UNIX_EPOCH - offset
    }
}

/// Downloads and snapshots in progress, in the synced folder so that they
/// reach their place with a rename. Made when first needed, so that a pass
/// with nothing to do touches nothing a watcher would report.
pub(super) struct Staging {
    dir: PathBuf,
}

impl Staging {
    pub fn new(root: &Path) -> Self {
        Self {
            dir: root.join(STAGING),
        }
    }

    /// What a pass cut short left behind.
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn file(&self) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.dir)?;
        Ok(self.dir.join(hex::encode(random_bytes::<8>())))
    }

    /// Removes the folder once empty.
    pub fn finish(&self) {
        let _ = fs::remove_dir(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_never_replace() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let refused = rename_new(&a, &b).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&b).unwrap(), b"b");

        let c = dir.path().join("c");
        rename_new(&a, &c).unwrap();
        assert_eq!(fs::read(&c).unwrap(), b"a");
        assert!(!a.exists());
    }

    #[test]
    fn replacing_checks_what_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("new"), dir.path().join("target"));
        fs::write(&target, b"seen").unwrap();
        let seen = scan::probe(&target).unwrap().unwrap().1;
        fs::write(&new, b"downloaded").unwrap();
        fs::write(&target, b"edited meanwhile").unwrap();
        assert!(!replace(&new, &target, seen).unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"edited meanwhile");

        let seen = scan::probe(&target).unwrap().unwrap().1;
        assert!(replace(&new, &target, seen).unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"downloaded");
        assert!(!new.exists());
    }

    #[test]
    fn times_round_trip_in_milliseconds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"f").unwrap();
        set_times(&path, Some(1_700_000_000_123), None).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let nanos = mtime.duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64;
        assert_eq!(millis(nanos), 1_700_000_000_123);
        assert_eq!(millis(-1), -1);
    }
}
