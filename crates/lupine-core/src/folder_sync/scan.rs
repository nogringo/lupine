//! The local side of a pass: what the folder holds now.

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_normalization::UnicodeNormalization;

use super::{Stat, ignored};
use crate::error::{Error, Result};
use crate::names;

#[derive(Debug)]
pub(super) struct Scanned {
    /// In the folder, `/`-separated, NFC.
    pub path: String,
    pub folder: bool,
    pub stat: Stat,
    /// Unix milliseconds.
    pub created: Option<i64>,
}

impl Scanned {
    pub fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(&self.path, |(_, name)| name)
    }

    /// `None` at the top of the folder.
    pub fn parent(&self) -> Option<&str> {
        self.path.rsplit_once('/').map(|(parent, _)| parent)
    }
}

#[derive(Debug, Default)]
pub(super) struct Scan {
    /// The file id of the folder itself.
    pub root: u64,
    /// Parents before their children.
    pub entries: Vec<Scanned>,
    /// What these hold, or for files what they are, could not be read: it
    /// is unknown, not gone.
    pub unknown: Vec<String>,
    /// Entries the drive cannot hold, such as links and other volumes. They
    /// stay on this device, and keep the folders that hold them there.
    pub skipped: Vec<String>,
}

impl Scan {
    /// `path` is `unknown` or under it.
    pub fn is_unknown(&self, path: &str) -> bool {
        self.unknown.iter().any(|u| {
            u.is_empty()
                || path
                    .strip_prefix(u.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }
}

/// Lists `root` without following links or crossing into other volumes.
/// Skips names the drive cannot hold. Fails when `root` is not a readable
/// folder.
pub(super) fn scan(root: &Path) -> Result<Scan> {
    let meta = fs::metadata(root).map_err(|_| Error::LocalFolderMissing)?;
    if !meta.is_dir() {
        return Err(Error::LocalFolderMissing);
    }
    let device = volume(&meta);
    let mut scan = Scan {
        root: file_id(&meta),
        ..Scan::default()
    };
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        let listing = match fs::read_dir(root.join(&dir)) {
            Ok(listing) => listing,
            Err(_) if dir.is_empty() => return Err(Error::LocalFolderMissing),
            Err(_) => {
                scan.unknown.push(dir);
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in listing {
            let Ok(entry) = entry else {
                scan.unknown.push(dir.clone());
                break;
            };
            let file_name = entry.file_name();
            let name: String = file_name.to_string_lossy().nfc().collect();
            if ignored(&name) {
                continue;
            }
            let path = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            if file_name.to_str().is_none() || names::validate(&name).is_err() {
                scan.skipped.push(path);
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                scan.unknown.push(path);
                continue;
            };
            let kind = meta.file_type();
            let folder = if kind.is_dir() && volume(&meta) == device {
                true
            } else if kind.is_file() {
                false
            } else {
                // Links, devices, other volumes.
                scan.skipped.push(path);
                continue;
            };
            children.push(Scanned {
                path,
                folder,
                stat: stat(&meta),
                created: meta.created().ok().map(|t| nanos(t).div_euclid(1_000_000)),
            });
        }
        children.sort_by(|a, b| a.path.cmp(&b.path));
        pending.extend(
            children
                .iter()
                .rev()
                .filter(|c| c.folder)
                .map(|c| c.path.clone()),
        );
        scan.entries.extend(children);
    }
    Ok(scan)
}

/// What is at `path` now, without following links. `None` when nothing is.
pub(super) fn probe(path: &Path) -> io::Result<Option<(bool, Stat)>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some((meta.is_dir(), stat(&meta)))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Folders keep only their file id: their times change with what they hold.
pub(super) fn stat(meta: &Metadata) -> Stat {
    if meta.is_dir() {
        return Stat {
            file_id: file_id(meta),
            size: 0,
            mtime: 0,
        };
    }
    Stat {
        file_id: file_id(meta),
        size: meta.len(),
        mtime: meta.modified().map(nanos).unwrap_or(0),
    }
}

fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i64,
        Err(before) => -(before.duration().as_nanos() as i64),
    }
}

#[cfg(unix)]
fn file_id(meta: &Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(meta)
}

#[cfg(unix)]
fn volume(meta: &Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(meta)
}

/// Without file ids, moves show as a deletion and a creation.
#[cfg(not(unix))]
fn file_id(_: &Metadata) -> u64 {
    0
}

#[cfg(not(unix))]
fn volume(_: &Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_parents_first_and_skips_what_the_drive_cannot_hold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("b/c")).unwrap();
        fs::write(root.join("b/c/d.txt"), b"d").unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join(".DS_Store"), b"").unwrap();
        fs::create_dir(root.join(super::super::STAGING)).unwrap();
        fs::write(root.join("cafe\u{301}.txt"), b"nfd").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();

        let scan = scan(root).unwrap();
        let paths: Vec<&str> = scan.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b", "caf\u{e9}.txt", "b/c", "b/c/d.txt"]);
        let file = &scan.entries[4];
        assert_eq!((file.name(), file.parent()), ("d.txt", Some("b/c")));
        assert_eq!(file.stat.size, 1);
        assert!(scan.entries[1].folder);
        assert!(scan.unknown.is_empty());
        #[cfg(unix)]
        assert_eq!(scan.skipped, ["link"]);
        assert!(matches!(
            super::scan(&root.join("a.txt")),
            Err(Error::LocalFolderMissing)
        ));
    }

    #[test]
    fn unknown_covers_what_is_under() {
        let scan = Scan {
            unknown: vec!["a/b".into()],
            ..Scan::default()
        };
        assert!(scan.is_unknown("a/b"));
        assert!(scan.is_unknown("a/b/c"));
        assert!(!scan.is_unknown("a/bc"));
        assert!(!scan.is_unknown("a"));
    }
}
