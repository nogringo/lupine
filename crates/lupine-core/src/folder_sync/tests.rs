use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::blob::MemoryBlobStore;
use crate::content::{self, MIN_CHUNK_SIZE};
use crate::crypto::random_bytes;
use crate::graph::History;
use crate::ids::DriveId;
use crate::io::MemoryFile;
use crate::items::Item;
use crate::session::DriveSession;
use crate::store::DriveRecord;
use crate::testing::TestRelay;
use crate::writer::{Contents, Edit, ItemWriter};

const OPTIONS: TransferOptions = TransferOptions {
    chunk_size: MIN_CHUNK_SIZE,
    concurrency: 2,
};

/// Keeps what it trashes, so tests can look.
struct Bin(PathBuf);

impl Trash for Bin {
    fn trash(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(&self.0)?;
        let name = path.file_name().unwrap().to_string_lossy();
        let to = self
            .0
            .join(format!("{name}.{}", hex::encode(random_bytes::<4>())));
        fs::rename(path, to)
    }
}

/// One device: its session, folder sync, and a File Provider extension
/// that stands for changes made on the drive.
struct Device {
    session: DriveSession,
    sync: FolderSync<Arc<MemoryBlobStore>>,
    finder: ItemWriter<Arc<MemoryBlobStore>>,
    dir: TempDir,
    folder: Option<SyncedFolder>,
    sources: usize,
}

impl Device {
    async fn new(drive: &DriveKeys, relay: &TestRelay, blobs: &Arc<MemoryBlobStore>) -> Self {
        let keys = || DriveKeys::from_secret(drive.keys().secret_key().clone());
        let store = Store::memory().unwrap();
        store
            .set_drive(&DriveRecord {
                id: DriveId([7; 16]),
                pubkey: drive.public_key(),
                epoch: 0,
                relays: vec![relay.url.clone()],
                servers: Vec::new(),
            })
            .unwrap();
        let store = Arc::new(store);
        let mut session = DriveSession::open(Arc::clone(&store), keys())
            .await
            .unwrap();
        session.sync().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bin = Bin(dir.path().join("bin"));
        let sync = FolderSync::new(Arc::clone(&store), keys(), Arc::clone(blobs), Box::new(bin))
            .unwrap()
            .with_options(OPTIONS);
        let finder = ItemWriter::new(store, keys(), Arc::clone(blobs))
            .unwrap()
            .with_options(OPTIONS);
        Self {
            session,
            sync,
            finder,
            dir,
            folder: None,
            sources: 0,
        }
    }

    /// Makes the local folder `Notes` with `files`, and syncs it.
    async fn add(&mut self, files: &[(&str, &[u8])]) -> FolderReport {
        let path = self.dir.path().join("Notes");
        fs::create_dir_all(&path).unwrap();
        for (name, bytes) in files {
            self.write(&path, name, bytes);
        }
        let folder = self.sync.add(&path, None).await.unwrap();
        self.session.take_in().await.unwrap();
        self.folder = Some(folder);
        self.pass().await
    }

    fn folder(&self) -> &SyncedFolder {
        self.folder.as_ref().unwrap()
    }

    fn root(&self) -> NodeId {
        self.folder().root
    }

    fn path(&self, name: &str) -> PathBuf {
        self.folder().path.join(name)
    }

    fn write(&self, root: &Path, name: &str, bytes: &[u8]) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if name.ends_with('/') {
            fs::create_dir_all(&path).unwrap();
        } else {
            fs::write(path, bytes).unwrap();
        }
    }

    fn local(&self, name: &str, bytes: &[u8]) {
        self.write(&self.folder().path, name, bytes);
    }

    /// A pass, then the session takes in what it wrote, as the app does.
    async fn pass(&mut self) -> FolderReport {
        let folder = self.folder().clone();
        let report = self.sync.sync(&folder).await.unwrap();
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        self.session.take_in().await.unwrap();
        report
    }

    /// Passes until nothing changes.
    async fn settle(&mut self) {
        for _ in 0..5 {
            self.session.sync().await.unwrap();
            self.session.settle().await.unwrap();
            if !self.pass().await.changed() {
                return;
            }
        }
        panic!("the folder does not settle");
    }

    fn local_listing(&self) -> Vec<String> {
        let scan = scan::scan(&self.folder().path).unwrap();
        let mut paths: Vec<String> = scan
            .entries
            .into_iter()
            .map(|e| {
                if e.folder {
                    format!("{}/", e.path)
                } else {
                    e.path
                }
            })
            .collect();
        paths.sort();
        paths
    }

    fn drive_listing(&self) -> Vec<String> {
        let store = self.sync.store();
        let items = store.subtree(self.root()).unwrap();
        let mut paths = Vec::new();
        for item in &items {
            let mut names = vec![item.name.clone()];
            let mut parent = item.parent.unwrap();
            while parent != self.root() {
                let up = store.item(&parent).unwrap().unwrap();
                names.push(up.name);
                parent = up.parent.unwrap();
            }
            names.reverse();
            let path = names.join("/");
            paths.push(if item.is_folder() {
                format!("{path}/")
            } else {
                path
            });
        }
        paths.sort();
        paths
    }

    fn item(&self, path: &str) -> Item {
        let mut parent = Some(self.root());
        let mut found = None;
        for name in path.split('/') {
            let children = self.sync.store().all_children(parent).unwrap();
            let item = children
                .into_iter()
                .find(|i| i.name == name)
                .unwrap_or_else(|| panic!("{path} is not on the drive"));
            parent = Some(item.node);
            found = Some(item);
        }
        found.unwrap()
    }

    async fn read(&self, item: &Item) -> Vec<u8> {
        let file = item.file.as_ref().unwrap();
        let (blobs, key) = (self.finder.blobs(), file.file_key());
        let content = file.content.resolve(blobs, &key).await.unwrap();
        let sink = Arc::new(MemoryFile::default());
        let all = 0..file.content.size;
        content::download(
            blobs,
            &content,
            &key,
            all,
            Arc::clone(&sink),
            OPTIONS,
            &|_| {},
        )
        .await
        .unwrap();
        sink.snapshot()
    }

    fn heads(&self, item: &Item) -> usize {
        let versions = self.sync.store().node_versions(&item.node).unwrap();
        History::new(versions.iter().map(|(id, v)| (*id, v)))
            .heads()
            .unwrap()
            .len()
    }

    fn source(&mut self, bytes: &[u8]) -> PathBuf {
        self.sources += 1;
        let path = self.dir.path().join(format!("source-{}", self.sources));
        fs::write(&path, bytes).unwrap();
        path
    }

    /// In the synced folder, or at its top when `parent` is `None`.
    async fn finder_create(
        &mut self,
        parent: Option<NodeId>,
        name: &str,
        bytes: Option<&[u8]>,
    ) -> Item {
        let parent = parent.or(Some(self.root()));
        self.finder_at(parent, name, bytes).await
    }

    /// `None` for the drive root. A folder without `bytes`.
    async fn finder_at(
        &mut self,
        parent: Option<NodeId>,
        name: &str,
        bytes: Option<&[u8]>,
    ) -> Item {
        let source = bytes.map(|bytes| self.source(bytes));
        let new = NewItem {
            request: format!("{parent:?}/{name}"),
            place: Place {
                parent,
                name: name.into(),
            },
            folder: bytes.is_none(),
            may_exist: false,
            created: None,
            modified: None,
        };
        let item = self
            .finder
            .create(new, source.as_deref().map(Contents::File), &|_| {})
            .await;
        item.unwrap().unwrap()
    }

    async fn finder_edit(&mut self, item: &Item, bytes: &[u8]) {
        let source = self.source(bytes);
        let edit = Edit {
            base_content: item.content_version(),
            ..Edit::default()
        };
        self.finder
            .modify(
                item.node,
                false,
                edit,
                Some(Contents::File(&source)),
                &|_| {},
            )
            .await
            .unwrap();
    }

    async fn finder_move(&self, item: &Item, parent: Option<NodeId>, name: &str) {
        let edit = Edit {
            base_content: item.content_version(),
            place: Some(Place {
                parent: parent.or(Some(self.root())),
                name: name.into(),
            }),
            ..Edit::default()
        };
        self.finder
            .modify(item.node, item.is_folder(), edit, None, &|_| {})
            .await
            .unwrap();
    }

    fn finder_delete(&self, item: &Item) {
        self.finder
            .delete(item.node, item.content_version(), true)
            .unwrap();
    }

    fn trashed(&self) -> usize {
        fs::read_dir(self.dir.path().join("bin")).map_or(0, |d| d.count())
    }
}

async fn device(relay: &TestRelay) -> (Device, DriveKeys, Arc<MemoryBlobStore>) {
    let drive = DriveKeys::generate();
    let blobs = Arc::new(MemoryBlobStore::new());
    (Device::new(&drive, relay, &blobs).await, drive, blobs)
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(ToString::to_string).collect()
}

#[tokio::test]
async fn a_new_folder_goes_up_once() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    let big: Vec<u8> = (0..3 * MIN_CHUNK_SIZE as usize + 5)
        .map(|i| (i % 251) as u8)
        .collect();
    let report = d
        .add(&[
            ("a.txt", b"a"),
            ("sub/b.bin", &big),
            ("empty/", b""),
            ("zero.txt", b""),
        ])
        .await;
    assert_eq!((report.uploaded, report.written), (3, 5));

    let listing = strings(&["a.txt", "empty/", "sub/", "sub/b.bin", "zero.txt"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
    assert_eq!(d.read(&d.item("sub/b.bin")).await, big);
    assert_eq!(
        d.item("a.txt").modified,
        Some(
            fs::metadata(d.path("a.txt"))
                .unwrap()
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
        )
    );
    assert!(!d.path(STAGING).exists(), "nothing left in staging");

    let again = d.pass().await;
    assert!(!again.changed(), "{again:?}");
}

#[tokio::test]
async fn changes_on_the_drive_reach_the_folder() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("a.txt", b"a"), ("sub/b.txt", b"b"), ("old.txt", b"old")])
        .await;

    let a = d.item("a.txt");
    d.finder_edit(&a, b"a, edited on the drive").await;
    let b = d.item("sub/b.txt");
    d.finder_move(&b, None, "b2.txt").await;
    d.finder_delete(&d.item("sub"));
    let new = d.finder_create(None, "new", None).await;
    d.finder_create(Some(new.node), "c.txt", Some(b"c")).await;
    let old = d.item("old.txt");
    d.finder_move(&old, None, "renamed.txt").await;
    d.session.take_in().await.unwrap();

    let report = d.pass().await;
    assert_eq!(report.written, 0);
    let listing = strings(&["a.txt", "b2.txt", "new/", "new/c.txt", "renamed.txt"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
    assert_eq!(
        fs::read(d.path("a.txt")).unwrap(),
        b"a, edited on the drive"
    );
    assert_eq!(fs::read(d.path("b2.txt")).unwrap(), b"b");
    assert_eq!(fs::read(d.path("new/c.txt")).unwrap(), b"c");
    assert_eq!(fs::read(d.path("renamed.txt")).unwrap(), b"old");
    assert_eq!(d.trashed(), 1, "sub went to the trash");
    assert!(!d.pass().await.changed());
}

#[tokio::test]
async fn changes_in_the_folder_reach_the_drive() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[
        ("a.txt", b"a"),
        ("sub/b.txt", b"b"),
        ("sub/c.txt", b"c"),
        ("old/x.txt", b"x"),
    ])
    .await;

    d.local("a.txt", b"a, edited here");
    fs::rename(d.path("sub"), d.path("Sub2")).unwrap();
    d.local("Sub2/new.txt", b"new");
    d.local("fresh/", b"");
    fs::rename(d.path("Sub2/b.txt"), d.path("fresh/b.txt")).unwrap();
    fs::remove_dir_all(d.path("old")).unwrap();

    let report = d.pass().await;
    // The edit, the rename, new.txt, fresh, the move of b.txt, and two
    // tombstones. Renaming sub writes nothing for what it holds.
    assert_eq!(report.written, 7, "{report:?}");
    let listing = strings(&[
        "Sub2/",
        "Sub2/c.txt",
        "Sub2/new.txt",
        "a.txt",
        "fresh/",
        "fresh/b.txt",
    ]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.read(&d.item("a.txt")).await, b"a, edited here");
    assert_eq!(d.read(&d.item("fresh/b.txt")).await, b"b");
    assert!(!d.pass().await.changed());
}

#[tokio::test]
async fn concurrent_edits_keep_both_contents() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("notes.txt", b"base")]).await;

    let notes = d.item("notes.txt");
    d.finder_edit(&notes, b"theirs").await;
    d.local("notes.txt", b"mine, longer");
    d.pass().await;
    assert_eq!(d.heads(&d.item("notes.txt")), 2, "in conflict");

    d.settle().await;
    let local = d.local_listing();
    assert_eq!(local.len(), 2, "{local:?}");
    assert_eq!(d.drive_listing(), local);
    let copy = local.iter().find(|p| p.contains("(conflict ")).unwrap();
    let contents: BTreeSet<Vec<u8>> = ["notes.txt", copy.as_str()]
        .iter()
        .map(|p| fs::read(d.path(p)).unwrap())
        .collect();
    assert_eq!(
        contents,
        BTreeSet::from([b"theirs".to_vec(), b"mine, longer".to_vec()])
    );
}

#[tokio::test]
async fn an_edit_wins_over_a_deletion() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("kept here.txt", b"a"), ("kept there.txt", b"b")])
        .await;

    d.finder_delete(&d.item("kept here.txt"));
    d.local("kept here.txt", b"edited here");
    let there = d.item("kept there.txt");
    d.finder_edit(&there, b"edited there").await;
    fs::remove_file(d.path("kept there.txt")).unwrap();
    d.session.take_in().await.unwrap();

    d.pass().await;
    let listing = strings(&["kept here.txt", "kept there.txt"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
    assert_eq!(d.read(&d.item("kept here.txt")).await, b"edited here");
    assert_eq!(fs::read(d.path("kept there.txt")).unwrap(), b"edited there");
    assert_eq!(d.trashed(), 0);
}

#[tokio::test]
async fn a_folder_deleted_here_keeps_what_the_drive_added() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("Photos/1.jpg", b"1"), ("Photos/2.jpg", b"2")])
        .await;

    fs::remove_dir_all(d.path("Photos")).unwrap();
    let photos = d.item("Photos");
    d.finder_create(Some(photos.node), "3.jpg", Some(b"3"))
        .await;
    d.session.take_in().await.unwrap();

    let report = d.pass().await;
    assert_eq!(report.written, 2, "1.jpg and 2.jpg");
    let listing = strings(&["Photos/", "Photos/3.jpg"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
}

#[tokio::test]
async fn deletions_here_follow_moves_made_on_the_drive() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("D/c.txt", b"c"), ("E/", b""), ("X/n.txt", b"n")])
        .await;

    let (c, e) = (d.item("D/c.txt"), d.item("E"));
    d.finder_move(&c, Some(e.node), "c.txt").await;
    let n = d.item("X/n.txt");
    d.finder_move(&n, None, "n.txt").await;
    d.finder_delete(&d.item("X"));
    d.session.take_in().await.unwrap();
    fs::remove_dir_all(d.path("D")).unwrap();
    fs::remove_file(d.path("X/n.txt")).unwrap();

    d.pass().await;
    assert_eq!(d.drive_listing(), strings(&["E/"]));
    assert_eq!(d.local_listing(), strings(&["E/"]));
    assert!(!d.pass().await.changed());
}

#[tokio::test]
async fn a_folder_moved_here_keeps_what_it_holds_when_the_drive_deletes_it() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("X/a.txt", b"a"), ("X/b.txt", b"b"), ("f.txt", b"f")])
        .await;

    d.finder_delete(&d.item("X"));
    d.session.take_in().await.unwrap();
    fs::rename(d.path("X"), d.path("Y")).unwrap();

    d.pass().await;
    let listing = strings(&["Y/", "Y/a.txt", "Y/b.txt", "f.txt"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
    assert_eq!(d.trashed(), 0);
    assert_eq!(d.read(&d.item("Y/b.txt")).await, b"b");
}

#[tokio::test]
async fn equal_size_and_time_are_not_taken_for_equal_content() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    let notes = d.finder_at(None, "Notes", None).await;
    let theirs = d
        .finder_at(Some(notes.node), "pkg.json", Some(b"theirs"))
        .await;
    d.session.take_in().await.unwrap();

    let path = d.dir.path().join("Notes");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("pkg.json"), b"minesX").unwrap();
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(path.join("pkg.json"))
        .unwrap()
        .set_modified(time)
        .unwrap();
    let edit = Edit {
        base_content: theirs.content_version(),
        modified: Some(1_000_000_000),
        ..Edit::default()
    };
    d.finder
        .modify(theirs.node, false, edit, None, &|_| {})
        .await
        .unwrap();
    d.add(&[]).await;

    assert_eq!(d.heads(&d.item("pkg.json")), 2, "in conflict");
    d.settle().await;
    let contents: BTreeSet<Vec<u8>> = d
        .local_listing()
        .iter()
        .map(|p| fs::read(d.path(p)).unwrap())
        .collect();
    assert_eq!(
        contents,
        BTreeSet::from([b"theirs".to_vec(), b"minesX".to_vec()])
    );
}

#[tokio::test]
async fn moves_that_make_a_loop_settle_inside_the_folder() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("A/a.txt", b"a"), ("B/b.txt", b"b")]).await;

    let (a, b) = (d.item("A"), d.item("B"));
    d.finder_move(&a, Some(b.node), "A").await;
    d.session.take_in().await.unwrap();
    fs::rename(d.path("B"), d.path("A/B")).unwrap();

    let folder = d.folder().clone();
    for _ in 0..3 {
        d.sync.sync(&folder).await.unwrap();
        d.session.take_in().await.unwrap();
    }
    let listing = strings(&["B/", "B/A/", "B/A/a.txt", "B/b.txt"]);
    assert_eq!(d.drive_listing(), listing);
    assert_eq!(d.local_listing(), listing);
    assert_eq!(d.trashed(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_name_held_by_what_is_never_synced_is_left_alone() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("a.txt", b"a")]).await;
    std::os::unix::fs::symlink("a.txt", d.path("current")).unwrap();
    d.finder_move(&d.item("a.txt"), None, "current").await;
    d.session.take_in().await.unwrap();

    let folder = d.folder().clone();
    for _ in 0..2 {
        let report = d.sync.sync(&folder).await.unwrap();
        assert_eq!(report.failed.len(), 1, "{report:?}");
    }
    assert_eq!(d.local_listing(), strings(&["a.txt"]), "not renamed aside");
}

#[cfg(unix)]
#[tokio::test]
async fn a_folder_holding_a_link_stays_when_the_drive_deletes_it() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("X/a.txt", b"a")]).await;
    std::os::unix::fs::symlink("a.txt", d.path("X/link")).unwrap();
    d.finder_delete(&d.item("X"));
    d.session.take_in().await.unwrap();

    d.pass().await;
    assert_eq!(d.local_listing(), strings(&["X/"]));
    assert!(fs::symlink_metadata(d.path("X/link")).is_ok());
    assert_eq!(d.trashed(), 1, "a.txt");
}

#[tokio::test]
async fn entries_new_on_both_sides_pair_up() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    let notes = d.finder_at(None, "Notes", None).await;
    d.finder_at(Some(notes.node), "same.txt", Some(b"same"))
        .await;
    d.finder_at(Some(notes.node), "differs.txt", Some(b"theirs"))
        .await;
    let docs = d.finder_at(Some(notes.node), "docs", None).await;
    d.finder_at(Some(docs.node), "remote.txt", Some(b"r")).await;
    d.session.take_in().await.unwrap();

    d.add(&[
        ("same.txt", b"same"),
        ("differs.txt", b"mine, longer"),
        ("docs/local.txt", b"l"),
    ])
    .await;
    assert_eq!(d.root(), notes.node, "the drive folder of the same name");
    d.settle().await;

    let local = d.local_listing();
    assert_eq!(d.drive_listing(), local);
    assert_eq!(local.len(), 6, "{local:?}");
    for path in [
        "docs/",
        "docs/local.txt",
        "docs/remote.txt",
        "same.txt",
        "differs.txt",
    ] {
        assert!(local.contains(&path.to_owned()), "{path} in {local:?}");
    }
    assert_eq!(
        d.heads(&d.item("same.txt")),
        1,
        "equal contents are no conflict"
    );
    assert_eq!(fs::read(d.path("docs/remote.txt")).unwrap(), b"r");
}

#[tokio::test]
async fn swapped_names_settle() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("a.txt", b"first"), ("b.txt", b"second")]).await;

    let (a, b) = (d.item("a.txt"), d.item("b.txt"));
    d.finder_move(&a, None, "swap").await;
    d.finder_move(&b, None, "a.txt").await;
    d.finder_move(&a, None, "b.txt").await;
    d.session.take_in().await.unwrap();

    d.settle().await;
    assert_eq!(d.local_listing(), strings(&["a.txt", "b.txt"]));
    assert_eq!(fs::read(d.path("a.txt")).unwrap(), b"second");
    assert_eq!(fs::read(d.path("b.txt")).unwrap(), b"first");
    assert_eq!(d.item("a.txt").node, b.node, "moved, not copied");
}

#[cfg(unix)]
#[tokio::test]
async fn what_cannot_be_read_is_not_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("locked/x.txt", b"x"), ("a.txt", b"a")]).await;

    let locked = d.path("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        // Running as root: nothing is unreadable.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    d.finder_delete(&d.item("a.txt"));
    d.session.take_in().await.unwrap();
    let report = d.pass().await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!((report.written, report.applied), (0, 1), "{report:?}");
    assert_eq!(d.drive_listing(), strings(&["locked/", "locked/x.txt"]));
    assert!(d.path("locked/x.txt").exists());
}

#[tokio::test]
async fn a_missing_folder_changes_nothing() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("a.txt", b"a")]).await;
    let folder = d.folder().clone();

    let away = d.dir.path().join("away");
    fs::rename(&folder.path, &away).unwrap();
    let missing = d.sync.sync(&folder).await;
    assert!(
        matches!(missing, Err(Error::LocalFolderMissing)),
        "{missing:?}"
    );
    assert_eq!(d.drive_listing(), strings(&["a.txt"]));

    fs::create_dir(&folder.path).unwrap();
    let replaced = d.sync.sync(&folder).await;
    assert!(
        matches!(replaced, Err(Error::LocalFolderMissing)),
        "another folder"
    );
    assert_eq!(d.drive_listing(), strings(&["a.txt"]));

    fs::remove_dir(&folder.path).unwrap();
    fs::rename(&away, &folder.path).unwrap();
    d.finder_delete(&d.sync.store().item(&folder.root).unwrap().unwrap());
    let gone = d.sync.sync(&folder).await;
    assert!(matches!(gone, Err(Error::DriveFolderMissing)), "{gone:?}");
    assert!(d.path("a.txt").exists());
}

#[tokio::test]
async fn folders_neither_nest_nor_share_a_drive_folder() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[("inner/a.txt", b"a")]).await;
    let nested = d.sync.add(&d.path("inner"), None).await;
    assert!(matches!(nested, Err(Error::FolderOverlap(_))));
    let outer = d.sync.add(d.dir.path(), None).await;
    assert!(matches!(outer, Err(Error::FolderOverlap(_))));

    let other = d.dir.path().join("elsewhere/Notes");
    fs::create_dir_all(&other).unwrap();
    let second = d.sync.add(&other, None).await.unwrap();
    assert_ne!(second.root, d.root());
    let name = d.sync.store().item(&second.root).unwrap().unwrap().name;
    assert_eq!(name, "Notes (2)");
}

#[tokio::test]
async fn two_devices_mirror_each_other() {
    let relay = TestRelay::start().await;
    let (mut one, drive, blobs) = device(&relay).await;
    one.add(&[("a.txt", b"a"), ("sub/b.txt", b"b")]).await;
    one.session.sync().await.unwrap();

    let mut two = Device::new(&drive, &relay, &blobs).await;
    two.add(&[]).await;
    assert_eq!(two.root(), one.root());
    assert_eq!(
        two.local_listing(),
        strings(&["a.txt", "sub/", "sub/b.txt"])
    );

    two.local("sub/b.txt", b"b, edited on two");
    fs::remove_file(two.path("a.txt")).unwrap();
    two.pass().await;
    two.session.sync().await.unwrap();

    one.session.sync().await.unwrap();
    one.pass().await;
    assert_eq!(one.local_listing(), strings(&["sub/", "sub/b.txt"]));
    assert_eq!(
        fs::read(one.path("sub/b.txt")).unwrap(),
        b"b, edited on two"
    );
    assert_eq!(one.trashed(), 1);
}

#[tokio::test]
async fn running_syncs_on_each_change_until_stopped() {
    let relay = TestRelay::start().await;
    let (mut d, ..) = device(&relay).await;
    d.add(&[]).await;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let changed = Notify::new();
    let (seen, mut synced) = tokio::sync::mpsc::unbounded_channel();
    let running = d.sync.run(
        async {
            stopped.await.ok();
        },
        &changed,
        |event| {
            if let FolderEvent::Synced(_, report) = event {
                seen.send(report.written).unwrap();
            }
        },
    );
    let driving = async {
        assert_eq!(synced.recv().await, Some(0));
        d.local("new.txt", b"new");
        changed.notify_one();
        assert_eq!(synced.recv().await, Some(1));
        stop.send(()).unwrap();
    };
    let (ran, ()) = tokio::join!(running, driving);
    ran.unwrap();
    assert_eq!(d.drive_listing(), strings(&["new.txt"]));
}
