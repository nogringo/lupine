//! One pass over a synced folder. The merge decides what each entry calls
//! for, from what the scan found, what the drive shows and what both last
//! agreed on. The changes then run in an order where each finds what it
//! depends on: local folders before what goes in them, moves out of a
//! folder before it goes to the trash, new drive folders before what moves
//! into them.
//!
//! Every change checks the state it was decided on, and leaves an entry for
//! the next pass when that state is gone. The record of what both sides
//! agree on is written after each change: a pass cut short in between finds
//! both sides equal and records them.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use futures::{StreamExt, stream};

use super::fs::{Staging, millis, rename_new, replace, set_times, touch};
use super::scan::{self, Scan};
use super::{FolderReport, FolderSync, Stat, Synced, SyncedFolder, ignored};
use crate::blob::BlobStore;
use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::ids::NodeId;
use crate::items::Item;
use crate::names::{self, Folded};
use crate::task::blocking;
use crate::writer::{Deletion, Edit, NewItem, Place};

/// Files uploaded or downloaded at once.
const TRANSFERS: usize = 4;

/// Where an entry is on disk: in a node's folder, or in a folder that only
/// exists locally so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pos {
    Node(NodeId),
    Fresh(usize),
}

/// A local entry without a node, until the pass creates it on the drive or
/// pairs it with a node new there too.
struct Fresh {
    pos: Pos,
    name: String,
    folder: bool,
    stat: Stat,
    created: Option<i64>,
    node: Option<NodeId>,
}

/// Both sides agree already: only the record changes.
struct Agreed {
    node: NodeId,
    folder: bool,
    content: Option<[u8; 32]>,
    modified: Option<i64>,
    stat: Stat,
}

#[derive(Clone, Copy)]
enum Upload {
    /// New content for `node`, made from the drive's `base`. `revive`
    /// writes the local place too, for a node gone from the drive folder.
    Node {
        node: NodeId,
        base: Option<[u8; 32]>,
        revive: bool,
    },
    Fresh(usize),
}

#[derive(Clone, Copy)]
enum Job {
    Download(NodeId),
    Upload(Upload),
}

#[derive(Default)]
struct Plan {
    forget: Vec<NodeId>,
    /// Parents first.
    mkdir: Vec<NodeId>,
    local_moves: Vec<NodeId>,
    /// With what they hold.
    trash: Vec<NodeId>,
    /// Parents first.
    create: Vec<usize>,
    /// Gone from the drive folder, kept by this device without new content.
    revive: Vec<NodeId>,
    remote_moves: Vec<NodeId>,
    /// With what they hold.
    delete: Vec<NodeId>,
    /// The drive content of every node a deletion may take.
    deletable: HashMap<NodeId, Option<[u8; 32]>>,
    downloads: Vec<NodeId>,
    uploads: Vec<Upload>,
    touch: Vec<NodeId>,
    agreed: Vec<Agreed>,
}

/// A transfer that went through.
struct Done {
    node: NodeId,
    content: Option<[u8; 32]>,
    modified: Option<i64>,
    stat: Stat,
    here: Option<(Pos, String)>,
    there: Option<(NodeId, String)>,
    fresh: Option<usize>,
    uploaded: bool,
    /// A version went to the drive.
    wrote: bool,
}

pub(super) async fn run<S: BlobStore + 'static>(
    sync: &FolderSync<S>,
    folder: &SyncedFolder,
) -> Result<FolderReport> {
    match sync.store.item(&folder.root)? {
        Some(root) if root.is_folder() => {}
        _ => return Err(Error::DriveFolderMissing),
    }
    let scan = scan_folder(folder).await?;
    let base = sync.store.synced(folder.id)?;
    let remote = sync.store.subtree(folder.root)?;
    let mut pass = Pass::new(sync, folder, base.clone(), remote.clone(), &scan);
    // Something moved while the scan ran shows as deleted: a second scan
    // finds it.
    if pass.lost_any() {
        let scan = scan_folder(folder).await?;
        pass = Pass::new(sync, folder, base, remote, &scan);
    }
    let plan = pass.decide();
    pass.staging.clear()?;
    let applied = pass.apply(plan).await;
    pass.staging.finish();
    applied?;
    Ok(pass.report)
}

async fn scan_folder(folder: &SyncedFolder) -> Result<Scan> {
    let path = folder.path.clone();
    let scan = blocking(move || scan::scan(&path)).await?;
    if scan.root != folder.file_id {
        return Err(Error::LocalFolderMissing);
    }
    Ok(scan)
}

struct Pass<'a, S> {
    sync: &'a FolderSync<S>,
    folder: &'a SyncedFolder,
    root: NodeId,
    base: HashMap<NodeId, Synced>,
    remote: HashMap<NodeId, Item>,
    /// By node id.
    remote_children: HashMap<NodeId, Vec<NodeId>>,
    /// Nodes on disk, as the scan saw them.
    found: HashMap<NodeId, Stat>,
    fresh: Vec<Fresh>,
    /// Where each node is on disk, as the pass changes it.
    here: HashMap<NodeId, (Pos, String)>,
    /// Where each node is on the drive, as the pass changes it.
    there: HashMap<NodeId, (NodeId, String)>,
    /// Recorded nodes the scan could not read: left as they are.
    frozen: HashSet<NodeId>,
    /// Nodes holding what the scan could not read or the drive cannot
    /// hold: never trashed.
    blocked: HashSet<NodeId>,
    staging: Staging,
    report: FolderReport,
}

impl<'a, S: BlobStore + 'static> Pass<'a, S> {
    fn new(
        sync: &'a FolderSync<S>,
        folder: &'a SyncedFolder,
        base: Vec<Synced>,
        remote: Vec<Item>,
        scan: &Scan,
    ) -> Self {
        let root = folder.root;
        let mut remote_map = HashMap::new();
        let mut remote_children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut there = HashMap::new();
        for item in remote {
            let Some(parent) = item.parent else {
                continue;
            };
            if ignored(&item.name) || (parent != root && !remote_map.contains_key(&parent)) {
                continue;
            }
            remote_children.entry(parent).or_default().push(item.node);
            there.insert(item.node, (parent, item.name.clone()));
            remote_map.insert(item.node, item);
        }
        for children in remote_children.values_mut() {
            children.sort();
        }
        let mut pass = Self {
            sync,
            folder,
            root,
            base: base.into_iter().map(|entry| (entry.node, entry)).collect(),
            remote: remote_map,
            remote_children,
            found: HashMap::new(),
            fresh: Vec::new(),
            here: HashMap::new(),
            there,
            frozen: HashSet::new(),
            blocked: HashSet::new(),
            staging: Staging::new(&folder.path),
            report: FolderReport::default(),
        };
        pass.locate(scan);
        pass
    }

    /// Finds each recorded node on disk: in place, moved (same file id), or
    /// replaced in place by a save that writes a new file. The rest is new.
    fn locate(&mut self, scan: &Scan) {
        let paths = self.base_paths();
        let by_path: HashMap<&str, NodeId> = paths.iter().map(|(n, p)| (p.as_str(), *n)).collect();
        let mut by_id: HashMap<(u64, bool), NodeId> = HashMap::new();
        for entry in self.base.values() {
            by_id
                .entry((entry.local.file_id, entry.folder))
                .and_modify(|n| *n = (*n).min(entry.node))
                .or_insert(entry.node);
        }
        let mut matched: Vec<Option<NodeId>> = vec![None; scan.entries.len()];
        let mut taken = HashSet::new();
        let kind = |node: &NodeId| self.base[node].folder;
        for (i, entry) in scan.entries.iter().enumerate() {
            if let Some(&node) = by_path.get(entry.path.as_str())
                && kind(&node) == entry.folder
                && self.base[&node].local.file_id == entry.stat.file_id
                && taken.insert(node)
            {
                matched[i] = Some(node);
            }
        }
        for (i, entry) in scan.entries.iter().enumerate() {
            if matched[i].is_none()
                && let Some(&node) = by_id.get(&(entry.stat.file_id, entry.folder))
                && taken.insert(node)
            {
                matched[i] = Some(node);
            }
        }
        for (i, entry) in scan.entries.iter().enumerate() {
            if matched[i].is_none()
                && let Some(&node) = by_path.get(entry.path.as_str())
                && kind(&node) == entry.folder
                && taken.insert(node)
            {
                matched[i] = Some(node);
            }
        }

        let mut pos_of: HashMap<&str, Pos> = HashMap::new();
        for (i, entry) in scan.entries.iter().enumerate() {
            let pos = match entry.parent() {
                None => Pos::Node(self.root),
                Some(parent) => match pos_of.get(parent) {
                    Some(pos) => *pos,
                    None => continue,
                },
            };
            let name = entry.name().to_owned();
            let at = match matched[i] {
                Some(node) => {
                    self.found.insert(node, entry.stat);
                    self.here.insert(node, (pos, name));
                    Pos::Node(node)
                }
                None => {
                    self.fresh.push(Fresh {
                        pos,
                        name,
                        folder: entry.folder,
                        stat: entry.stat,
                        created: entry.created,
                        node: None,
                    });
                    Pos::Fresh(self.fresh.len() - 1)
                }
            };
            pos_of.insert(&entry.path, at);
        }

        for (node, path) in &paths {
            if !self.found.contains_key(node) && scan.is_unknown(path) {
                self.frozen.insert(*node);
            }
        }
        for kept in scan.unknown.iter().chain(&scan.skipped) {
            let mut current = Some(kept.as_str());
            while let Some(path) = current {
                if let Some(Pos::Node(node)) = pos_of.get(path) {
                    self.blocked.insert(*node);
                }
                current = path.rsplit_once('/').map(|(parent, _)| parent);
            }
        }
    }

    /// Where each recorded node was when both sides last agreed. A node
    /// whose parents do not lead to the root has none.
    fn base_paths(&self) -> HashMap<NodeId, String> {
        let mut paths: HashMap<NodeId, String> = HashMap::new();
        for &node in self.base.keys() {
            let mut chain = Vec::new();
            let mut current = node;
            let prefix = loop {
                if current == self.root {
                    break Some(String::new());
                }
                if let Some(path) = paths.get(&current) {
                    break Some(path.clone());
                }
                match self.base.get(&current) {
                    Some(entry) if chain.len() <= self.base.len() => {
                        chain.push(current);
                        current = entry.parent;
                    }
                    _ => break None,
                }
            };
            let Some(mut path) = prefix else {
                continue;
            };
            for n in chain.iter().rev() {
                let name = &self.base[n].name;
                path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}/{name}")
                };
                paths.insert(*n, path.clone());
            }
        }
        paths
    }

    /// Recorded nodes the scan did not find.
    fn lost_any(&self) -> bool {
        self.base
            .keys()
            .any(|node| !self.found.contains_key(node) && !self.frozen.contains(node))
    }

    fn decide(&mut self) -> Plan {
        let mut plan = Plan::default();
        let mut nodes: Vec<NodeId> = self.base.keys().copied().collect();
        nodes.sort();
        let (mut local_gone, mut remote_gone, mut both) = (Vec::new(), Vec::new(), Vec::new());
        for node in nodes {
            if self.frozen.contains(&node) {
                continue;
            }
            match (self.found.contains_key(&node), self.remote.get(&node)) {
                (false, None) => plan.forget.push(node),
                (true, None) => remote_gone.push(node),
                (false, Some(_)) => local_gone.push(node),
                (true, Some(item)) if item.is_folder() == self.base[&node].folder => {
                    both.push(node)
                }
                // The drive changed what the node is: left alone.
                (true, Some(_)) => {}
            }
        }
        let targets: HashMap<NodeId, Pos> = both.iter().map(|n| (*n, self.target(n))).collect();
        self.decide_both(&both, &mut plan);
        self.decide_remote_gone(&remote_gone, &targets, &mut plan);
        self.decide_local_gone(&local_gone, &targets, &mut plan);
        self.pair(&both, &mut plan);
        plan
    }

    /// The folder a node present on both sides ends in: the drive's place
    /// wins over a local move.
    fn target(&self, node: &NodeId) -> Pos {
        let (base, (pos, name)) = (&self.base[node], &self.here[node]);
        let (parent, remote_name) = &self.there[node];
        let remote = (Pos::Node(*parent), remote_name.as_str());
        let moved_there = remote != (Pos::Node(base.parent), base.name.as_str());
        if (*pos, name.as_str()) == remote || moved_there {
            remote.0
        } else {
            *pos
        }
    }

    fn decide_both(&self, both: &[NodeId], plan: &mut Plan) {
        for &node in both {
            let base = &self.base[&node];
            let item = &self.remote[&node];
            let (pos, name) = &self.here[&node];
            let (parent, remote_name) = &self.there[&node];
            let local = (*pos, name.as_str());
            let remote = (Pos::Node(*parent), remote_name.as_str());
            let recorded = (Pos::Node(base.parent), base.name.as_str());
            if local != remote {
                if remote != recorded {
                    plan.local_moves.push(node);
                } else {
                    plan.remote_moves.push(node);
                }
            }
            let place_only = local == remote && local != recorded;
            if base.folder {
                if place_only {
                    plan.agreed.push(Agreed {
                        node,
                        folder: true,
                        content: None,
                        modified: None,
                        stat: self.found[&node],
                    });
                }
                continue;
            }
            let stat = self.found[&node];
            let local_changed = stat != base.local;
            let remote_changed = item.content_version() != base.content;
            match (local_changed, remote_changed) {
                (false, false) if item.modified != base.modified => plan.touch.push(node),
                (false, false) if place_only => plan.agreed.push(Agreed {
                    node,
                    folder: false,
                    content: base.content,
                    modified: base.modified,
                    stat,
                }),
                (false, false) => {}
                (false, true) => plan.downloads.push(node),
                // A conflict when the drive changed too, unless both
                // contents turn out equal: uploaded with the drive file's
                // key, equal content writes nothing.
                (true, _) => plan.uploads.push(Upload::Node {
                    node,
                    base: base.content,
                    revive: false,
                }),
            }
        }
    }

    /// Gone from the drive folder, still on disk: trashed, unless this
    /// device changed it, or something in it stays. Then it comes back on
    /// the drive.
    fn decide_remote_gone(&self, gone: &[NodeId], targets: &HashMap<NodeId, Pos>, plan: &mut Plan) {
        let gone_set: HashSet<NodeId> = gone.iter().copied().collect();
        let mut kids: HashMap<NodeId, Vec<Pos>> = HashMap::new();
        for (node, (pos, _)) in &self.here {
            if let Pos::Node(parent) = pos {
                kids.entry(*parent).or_default().push(Pos::Node(*node));
            }
        }
        for (i, fresh) in self.fresh.iter().enumerate() {
            if let Pos::Node(parent) = fresh.pos {
                kids.entry(parent).or_default().push(Pos::Fresh(i));
            }
        }
        let mut order = gone.to_vec();
        order.sort_by_key(|n| Reverse(self.local_depth(*n)));
        let mut kept = HashSet::new();
        for &node in &order {
            let base = &self.base[&node];
            let (pos, name) = &self.here[&node];
            let changed = (*pos, name.as_str()) != (Pos::Node(base.parent), base.name.as_str())
                || (!base.folder && self.found[&node] != base.local);
            let holds = kids.get(&node).into_iter().flatten().any(|kid| match kid {
                Pos::Fresh(_) => true,
                Pos::Node(m) if gone_set.contains(m) => kept.contains(m),
                Pos::Node(m) => targets.get(m) == Some(&Pos::Node(node)),
            });
            if changed || holds || self.blocked.contains(&node) {
                kept.insert(node);
            }
        }
        // A folder moved here comes back with all it holds.
        let mut whole = HashSet::new();
        for &node in order.iter().rev() {
            let base = &self.base[&node];
            let (pos, name) = &self.here[&node];
            let moved = (*pos, name.as_str()) != (Pos::Node(base.parent), base.name.as_str());
            let inside = matches!(pos, Pos::Node(parent) if whole.contains(parent));
            if inside || (base.folder && moved) {
                whole.insert(node);
                kept.insert(node);
            }
        }
        for &node in order.iter().rev() {
            if self.blocked.contains(&node) {
                continue;
            }
            let base = &self.base[&node];
            if kept.contains(&node) {
                if base.folder || self.found[&node] == base.local {
                    plan.revive.push(node);
                } else {
                    plan.uploads.push(Upload::Node {
                        node,
                        base: base.content,
                        revive: true,
                    });
                }
                continue;
            }
            let in_trashed = match self.here[&node].0 {
                Pos::Node(parent) => gone_set.contains(&parent) && !kept.contains(&parent),
                Pos::Fresh(_) => false,
            };
            if !in_trashed {
                plan.trash.push(node);
            }
        }
    }

    /// Gone from disk, still on the drive: deleted there, unless it
    /// changed there or something in it stays. Then it comes back here.
    fn decide_local_gone(&self, gone: &[NodeId], targets: &HashMap<NodeId, Pos>, plan: &mut Plan) {
        let gone_set: HashSet<NodeId> = gone.iter().copied().collect();
        let mut order = gone.to_vec();
        order.sort_by_key(|n| Reverse(self.remote_depth(*n)));
        let mut deletable = HashSet::new();
        for &node in &order {
            let base = &self.base[&node];
            let unchanged = if base.folder {
                self.remote_children
                    .get(&node)
                    .into_iter()
                    .flatten()
                    .all(|child| {
                        if gone_set.contains(child) {
                            deletable.contains(child)
                        } else {
                            targets.get(child).is_some_and(|t| *t != Pos::Node(node))
                        }
                    })
            } else {
                self.remote[&node].content_version() == base.content
            };
            if unchanged {
                deletable.insert(node);
                plan.deletable.insert(node, base.content);
            }
        }
        for &node in order.iter().rev() {
            let parent = self.there[&node].0;
            if deletable.contains(&node) {
                if !deletable.contains(&parent) {
                    plan.delete.push(node);
                }
            } else if self.base[&node].folder {
                plan.mkdir.push(node);
            } else {
                plan.downloads.push(node);
            }
        }
    }

    /// Entries new on either side, from the top down: those new on both at
    /// the same place, by name as a case-insensitive volume compares it,
    /// become one. A folder pairs with a folder, a file with a file.
    fn pair(&mut self, both: &[NodeId], plan: &mut Plan) {
        let mut fresh_kids: HashMap<Pos, Vec<usize>> = HashMap::new();
        for (i, fresh) in self.fresh.iter().enumerate() {
            fresh_kids.entry(fresh.pos).or_default().push(i);
        }
        let mut remote_new: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (parent, children) in &self.remote_children {
            let new: Vec<NodeId> = children
                .iter()
                .filter(|child| !self.base.contains_key(child))
                .copied()
                .collect();
            if !new.is_empty() {
                remote_new.insert(*parent, new);
            }
        }

        let mut work: Vec<(Option<Pos>, Option<NodeId>)> =
            vec![(Some(Pos::Node(self.root)), Some(self.root))];
        work.extend(
            both.iter()
                .filter(|n| self.base[n].folder)
                .map(|n| (Some(Pos::Node(*n)), Some(*n))),
        );
        work.extend(plan.mkdir.iter().map(|n| (None, Some(*n))));
        work.extend(plan.revive.iter().map(|n| (Some(Pos::Node(*n)), None)));

        while let Some((local, remote)) = work.pop() {
            let mut locals = local
                .and_then(|pos| fresh_kids.remove(&pos))
                .unwrap_or_default();
            let remotes = remote
                .and_then(|node| remote_new.remove(&node))
                .unwrap_or_default();
            for child in remotes {
                let item = &self.remote[&child];
                let (name, folder) = (Folded(item.name.clone()), item.is_folder());
                let partner = locals.iter().position(|&i| {
                    let fresh = &self.fresh[i];
                    fresh.folder == folder && Folded(fresh.name.clone()) == name
                });
                let Some(at) = partner else {
                    if folder {
                        plan.mkdir.push(child);
                        work.push((None, Some(child)));
                    } else {
                        plan.downloads.push(child);
                    }
                    continue;
                };
                let i = locals.remove(at);
                let fresh = &mut self.fresh[i];
                fresh.node = Some(child);
                let (stat, pos, name) = (fresh.stat, fresh.pos, fresh.name.clone());
                self.here.insert(child, (pos, name));
                if folder {
                    plan.agreed.push(Agreed {
                        node: child,
                        folder: true,
                        content: None,
                        modified: None,
                        stat,
                    });
                    work.push((Some(Pos::Fresh(i)), Some(child)));
                } else {
                    // Equal contents write nothing; others end in a
                    // conflict copy.
                    self.found.insert(child, stat);
                    plan.uploads.push(Upload::Node {
                        node: child,
                        base: None,
                        revive: false,
                    });
                }
            }
            locals.sort_unstable();
            for i in locals {
                if self.fresh[i].folder {
                    plan.create.push(i);
                    work.push((Some(Pos::Fresh(i)), None));
                } else {
                    plan.uploads.push(Upload::Fresh(i));
                }
            }
        }
    }

    async fn apply(&mut self, plan: Plan) -> Result<()> {
        if !plan.forget.is_empty() {
            self.sync
                .store
                .forget_synced(self.folder.id, &plan.forget)?;
        }
        if !self.still_synced()? {
            return Ok(());
        }
        let mut mkdir = self.mkdirs(plan.mkdir)?;
        let mut moves = self.local_moves(plan.local_moves)?;
        self.trash(&plan.trash)?;
        // The trash may have freed the names these wait for.
        mkdir = self.mkdirs(mkdir)?;
        moves = self.local_moves(moves)?;
        for node in mkdir.into_iter().chain(moves) {
            let name = self
                .there
                .get(&node)
                .map_or("the name", |(_, name)| name.as_str());
            let taken = Error::Io(io::Error::other(format!("{name} is taken")));
            self.skip_node(node, taken)?;
        }

        if !self.still_synced()? {
            return Ok(());
        }
        self.creates(&plan.create).await?;
        let mut uploads = plan.uploads;
        uploads.extend(self.revives(&plan.revive).await?);
        self.remote_moves(plan.remote_moves).await?;
        self.deletes(&plan.delete, &plan.deletable)?;

        if !self.still_synced()? {
            return Ok(());
        }
        self.transfers(plan.downloads, uploads).await?;
        self.touches(&plan.touch)?;
        for agreed in plan.agreed {
            self.record(
                agreed.node,
                agreed.folder,
                agreed.content,
                agreed.modified,
                agreed.stat,
            )?;
        }
        Ok(())
    }

    fn still_synced(&self) -> Result<bool> {
        Ok(self.sync.store.folder(self.folder.id)?.is_some())
    }

    /// Returns the folders whose name is taken.
    fn mkdirs(&mut self, nodes: Vec<NodeId>) -> Result<Vec<NodeId>> {
        let mut taken = Vec::new();
        for node in nodes {
            let Some((parent, name)) = self.there.get(&node).cloned() else {
                continue;
            };
            let Some(dir) = self.path(Pos::Node(parent)) else {
                self.skip_node(node, Error::UnknownParent)?;
                continue;
            };
            let path = dir.join(&name);
            let made = std::fs::create_dir(&path).and_then(|()| scan::probe(&path));
            match made {
                Ok(Some((true, stat))) => {
                    self.here.insert(node, (Pos::Node(parent), name));
                    self.report.applied += 1;
                    self.record(node, true, None, None, stat)?;
                }
                Ok(_) => self.skip_node(node, Error::SourceChanged)?,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => taken.push(node),
                Err(e) => self.skip_node(node, e.into())?,
            }
        }
        Ok(taken)
    }

    /// To the drive's place. Returns the moves left waiting: for their name,
    /// for their folder, or for a folder still inside them to leave. When
    /// none can go first, as in a swap of two names, one holding a name
    /// another waits for steps aside, and comes back if it cannot move on.
    fn local_moves(&mut self, nodes: Vec<NodeId>) -> Result<Vec<NodeId>> {
        let mut pending = nodes;
        let mut aside: HashMap<NodeId, String> = HashMap::new();
        loop {
            let before = pending.len();
            let mut waiting = Vec::new();
            let mut held = Vec::new();
            for node in pending {
                let Some((parent, name)) = self.there.get(&node).cloned() else {
                    continue;
                };
                let (Some(from), Some(dir)) =
                    (self.path(Pos::Node(node)), self.path(Pos::Node(parent)))
                else {
                    waiting.push(node);
                    continue;
                };
                let to = dir.join(&name);
                match rename_new(&from, &to) {
                    Ok(()) => {
                        self.here.insert(node, (Pos::Node(parent), name));
                        aside.remove(&node);
                        self.report.applied += 1;
                        self.record_place(node)?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        if let Ok(Some((_, stat))) = scan::probe(&to) {
                            held.push(stat.file_id);
                        }
                        waiting.push(node);
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                        ) =>
                    {
                        waiting.push(node)
                    }
                    Err(e) => self.skip_node(node, e.into())?,
                }
            }
            let stuck = !waiting.is_empty() && waiting.len() == before;
            pending = waiting;
            if !stuck {
                if pending.is_empty() {
                    break;
                }
                continue;
            }
            let holder = pending.iter().copied().find(|m| {
                !aside.contains_key(m)
                    && self
                        .found
                        .get(m)
                        .is_some_and(|stat| held.contains(&stat.file_id))
            });
            let Some(holder) = holder else {
                break;
            };
            let (Some((pos, name)), Some(from)) = (
                self.here.get(&holder).cloned(),
                self.path(Pos::Node(holder)),
            ) else {
                break;
            };
            let suffix = format!(" lupine-{}", hex::encode(random_bytes::<4>()));
            let temporary = names::with_suffix(&name, &suffix);
            if rename_new(&from, &from.with_file_name(&temporary)).is_err() {
                break;
            }
            self.here.insert(holder, (pos, temporary));
            aside.insert(holder, name);
        }
        for (node, name) in aside {
            if let (Some((pos, _)), Some(from)) =
                (self.here.get(&node).cloned(), self.path(Pos::Node(node)))
                && rename_new(&from, &from.with_file_name(&name)).is_ok()
            {
                self.here.insert(node, (pos, name));
            }
        }
        Ok(pending)
    }

    fn trash(&mut self, nodes: &[NodeId]) -> Result<()> {
        if nodes.is_empty() {
            return Ok(());
        }
        let mut recorded: HashMap<NodeId, Vec<&Synced>> = HashMap::new();
        for entry in self.base.values() {
            if !self.remote.contains_key(&entry.node) {
                recorded.entry(entry.parent).or_default().push(entry);
            }
        }
        let mut on_disk: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (node, (pos, _)) in &self.here {
            if let Pos::Node(parent) = pos {
                on_disk.entry(*parent).or_default().push(*node);
            }
        }
        let mut trashed = Vec::new();
        let mut failed = Vec::new();
        for &node in nodes {
            let Some(path) = self.path(Pos::Node(node)) else {
                continue;
            };
            let unchanged = match self.unchanged_on_disk(node, &path, &recorded) {
                Ok(true) => self.sync.trash.trash(&path).map_err(Error::from),
                Ok(false) => Err(Error::SourceChanged),
                Err(e) => Err(e),
            };
            match unchanged {
                Ok(()) => {
                    self.report.applied += 1;
                    trashed.extend(subtree(node, &on_disk));
                }
                Err(e) => failed.push((node, e)),
            }
        }
        for (node, error) in failed {
            self.skip_node(node, error)?;
        }
        for node in &trashed {
            self.here.remove(node);
        }
        self.forget(trashed)
    }

    /// The trash takes only what both sides last agreed on and the drive
    /// no longer holds: nothing added or changed since, nothing still in
    /// the drive folder, nothing the drive cannot hold.
    fn unchanged_on_disk(
        &self,
        node: NodeId,
        path: &Path,
        recorded: &HashMap<NodeId, Vec<&Synced>>,
    ) -> Result<bool> {
        let Some(base) = self.base.get(&node) else {
            return Ok(false);
        };
        if !base.folder {
            return Ok(scan::probe(path)? == Some((false, base.local)));
        }
        let mut expected: HashMap<String, &Synced> = HashMap::new();
        let mut pending = vec![(node, String::new())];
        while let Some((parent, prefix)) = pending.pop() {
            for child in recorded.get(&parent).into_iter().flatten() {
                let path = if prefix.is_empty() {
                    child.name.clone()
                } else {
                    format!("{prefix}/{}", child.name)
                };
                if expected.insert(path.clone(), child).is_none() {
                    pending.push((child.node, path));
                }
            }
        }
        let found = scan::scan(path)?;
        Ok(found.unknown.is_empty()
            && found.skipped.is_empty()
            && found.entries.iter().all(|entry| {
                expected.get(&entry.path).is_some_and(|synced| {
                    synced.folder == entry.folder && (entry.folder || synced.local == entry.stat)
                })
            }))
    }

    /// Drops the records of `nodes`, which neither side holds any more.
    fn forget(&mut self, nodes: Vec<NodeId>) -> Result<()> {
        let nodes: Vec<NodeId> = nodes
            .into_iter()
            .filter(|node| self.base.contains_key(node))
            .collect();
        self.sync.store.forget_synced(self.folder.id, &nodes)?;
        for node in nodes {
            self.base.remove(&node);
        }
        Ok(())
    }

    async fn creates(&mut self, fresh: &[usize]) -> Result<()> {
        for &i in fresh {
            let Some(parent) = self.node_of(self.fresh[i].pos) else {
                self.skip_fresh(i, Error::UnknownParent)?;
                continue;
            };
            let entry = &self.fresh[i];
            let (pos, name, stat) = (entry.pos, entry.name.clone(), entry.stat);
            let new = NewItem {
                request: NodeId::random().to_hex(),
                place: Place {
                    parent: Some(parent),
                    name: name.clone(),
                },
                folder: true,
                may_exist: false,
                created: entry.created,
                modified: None,
            };
            match self.sync.writer.create(new, None, &|_| {}).await {
                Ok(Some(item)) => {
                    self.fresh[i].node = Some(item.node);
                    self.here.insert(item.node, (pos, name.clone()));
                    self.there.insert(item.node, (parent, name));
                    self.report.written += 1;
                    self.record(item.node, true, None, None, stat)?;
                }
                Ok(None) => self.skip_fresh(i, Error::Malformed("no folder created"))?,
                Err(e) => self.skip_fresh(i, e)?,
            }
        }
        Ok(())
    }

    /// Puts back on the drive, at their place here, nodes its folder no
    /// longer shows. Returns the files the drive keeps no version of to put
    /// back: their content goes up again.
    async fn revives(&mut self, nodes: &[NodeId]) -> Result<Vec<Upload>> {
        let mut nodes = nodes.to_vec();
        nodes.sort_by_key(|n| self.local_depth(*n));
        let mut uploads = Vec::new();
        for node in nodes {
            let (Some(place), Some(base), Some(&stat)) = (
                self.local_place(node),
                self.base.get(&node).cloned(),
                self.found.get(&node),
            ) else {
                self.skip_node(node, Error::UnknownParent)?;
                continue;
            };
            let edit = Edit {
                place: Some(place.clone()),
                ..Edit::default()
            };
            let revived = self
                .sync
                .writer
                .modify(node, base.folder, edit, None, &|_| {});
            match revived.await {
                Ok(_) => {
                    self.there
                        .insert(node, (place.parent.unwrap_or(self.root), place.name));
                    self.report.written += 1;
                    // What the device holds: a later content the drive
                    // brings back comes down on the next pass.
                    self.record(node, base.folder, base.content, base.modified, stat)?;
                }
                Err(Error::UnknownNode) if !base.folder => uploads.push(Upload::Node {
                    node,
                    base: base.content,
                    revive: true,
                }),
                Err(e) => self.skip_node(node, e)?,
            }
        }
        Ok(uploads)
    }

    /// Shallowest first, so that no move puts a folder inside one that has
    /// yet to leave it. A move into a folder the drive moved into the node
    /// meanwhile would make a loop: the drive's place wins instead.
    async fn remote_moves(&mut self, mut nodes: Vec<NodeId>) -> Result<()> {
        nodes.sort_by_key(|n| self.local_depth(*n));
        for node in nodes {
            let (Some(place), Some(base)) = (self.local_place(node), self.base.get(&node).cloned())
            else {
                self.skip_node(node, Error::UnknownParent)?;
                continue;
            };
            let parent = place.parent.unwrap_or(self.root);
            if self.on_drive_under(parent, node) {
                for waiting in self.local_moves(vec![node])? {
                    let looped = Error::Io(io::Error::other("the drive moved its folder into it"));
                    self.skip_node(waiting, looped)?;
                }
                continue;
            }
            let edit = Edit {
                base_content: base.content,
                place: Some(place.clone()),
                ..Edit::default()
            };
            let moved = self
                .sync
                .writer
                .modify(node, base.folder, edit, None, &|_| {});
            match moved.await {
                Ok(_) => {
                    self.there.insert(node, (parent, place.name));
                    self.report.written += 1;
                    self.record_place(node)?;
                }
                Err(e) => self.skip_node(node, e)?,
            }
        }
        Ok(())
    }

    /// `node` is `ancestor` or in it, on the drive as the pass knows it.
    fn on_drive_under(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut current = node;
        for _ in 0..=self.there.len() {
            if current == ancestor {
                return true;
            }
            match self.there.get(&current) {
                Some((parent, _)) => current = *parent,
                None => return false,
            }
        }
        false
    }

    /// Each deletion takes only what the pass decided to delete: anything
    /// that reached the folder on the drive since, or changed there, keeps
    /// it back for the next pass to bring here.
    fn deletes(
        &mut self,
        nodes: &[NodeId],
        deletable: &HashMap<NodeId, Option<[u8; 32]>>,
    ) -> Result<()> {
        let mut on_drive: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (node, (parent, _)) in &self.there {
            on_drive.entry(*parent).or_default().push(*node);
        }
        for &node in nodes {
            let expected = |item: &Item| {
                ignored(&item.name) || deletable.get(&item.node) == Some(&item.content_version())
            };
            match self.sync.writer.delete_if(node, expected) {
                Ok(Deletion::Deleted(written)) => {
                    self.report.written += written;
                    self.forget(subtree(node, &on_drive))?;
                }
                Ok(Deletion::Unknown) => self.forget(subtree(node, &on_drive))?,
                Ok(Deletion::Kept(_)) => {}
                Err(e) => self.skip_node(node, e)?,
            }
        }
        Ok(())
    }

    async fn transfers(&mut self, downloads: Vec<NodeId>, uploads: Vec<Upload>) -> Result<()> {
        let jobs = downloads
            .into_iter()
            .map(Job::Download)
            .chain(uploads.into_iter().map(Job::Upload));
        let this = &*self;
        let outcomes: Vec<(Job, Result<Done>)> = stream::iter(jobs)
            .map(|job| async move {
                let done = match job {
                    Job::Download(node) => this.download(node).await,
                    Job::Upload(upload) => this.upload(upload).await,
                };
                (job, done)
            })
            .buffer_unordered(TRANSFERS)
            .collect()
            .await;

        for (job, outcome) in outcomes {
            let done = match outcome {
                Ok(done) => done,
                Err(e) => {
                    match job {
                        Job::Download(node) | Job::Upload(Upload::Node { node, .. }) => {
                            self.skip_node(node, e)?
                        }
                        Job::Upload(Upload::Fresh(i)) => self.skip_fresh(i, e)?,
                    }
                    continue;
                }
            };
            if let Some(i) = done.fresh {
                self.fresh[i].node = Some(done.node);
            }
            if let Some(here) = done.here {
                self.here.insert(done.node, here);
            }
            if let Some(there) = done.there {
                self.there.insert(done.node, there);
            }
            if done.wrote {
                self.report.uploaded += 1;
                self.report.written += 1;
            } else if !done.uploaded {
                self.report.downloaded += 1;
                self.report.applied += 1;
            }
            self.record(done.node, false, done.content, done.modified, done.stat)?;
        }
        Ok(())
    }

    /// Into the staging folder, then in place with a rename, provided the
    /// place still holds what the scan saw there.
    async fn download(&self, node: NodeId) -> Result<Done> {
        let item = self.remote.get(&node).ok_or(Error::UnknownNode)?;
        let file = item
            .file
            .as_ref()
            .ok_or(Error::Malformed("a file without content"))?;
        let (pos, name) = match (self.here.get(&node), self.there.get(&node)) {
            (Some(at), _) => at.clone(),
            (None, Some((parent, name))) => (Pos::Node(*parent), name.clone()),
            (None, None) => return Err(Error::UnknownNode),
        };
        let target = self.path(pos).ok_or(Error::UnknownParent)?.join(&name);
        let seen = self.found.get(&node).copied();
        let unchanged = match (scan::probe(&target)?, seen) {
            (None, None) => true,
            (Some((false, now)), Some(seen)) => now == seen,
            _ => false,
        };
        if !unchanged {
            return Err(Error::SourceChanged);
        }
        let temp = self.staging.file()?;
        let placed = async {
            self.sync
                .reader
                .read(file, 0..u64::MAX, 1, &temp, &|_| {})
                .await?;
            set_times(&temp, item.modified, item.created)?;
            let placed = match seen {
                None => rename_new(&temp, &target).map(|()| true)?,
                Some(seen) => replace(&temp, &target, seen)?,
            };
            if !placed {
                return Err(Error::SourceChanged);
            }
            match scan::probe(&target)? {
                Some((false, stat)) => Ok(stat),
                _ => Err(Error::SourceChanged),
            }
        }
        .await;
        if placed.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        Ok(Done {
            node,
            content: item.content_version(),
            modified: item.modified,
            stat: placed?,
            here: Some((pos, name)),
            there: None,
            fresh: None,
            uploaded: false,
            wrote: false,
        })
    }

    async fn upload(&self, upload: Upload) -> Result<Done> {
        match upload {
            Upload::Node { node, base, revive } => {
                let source = self.path(Pos::Node(node)).ok_or(Error::UnknownNode)?;
                let seen = *self.found.get(&node).ok_or(Error::UnknownNode)?;
                let place = match revive {
                    true => Some(self.local_place(node).ok_or(Error::UnknownParent)?),
                    false => None,
                };
                let edit = Edit {
                    base_content: base,
                    place: place.clone(),
                    created: None,
                    modified: Some(millis(seen.mtime)),
                };
                let snapshot = self.snapshot(&source, seen).await?;
                let written = self
                    .sync
                    .writer
                    .modify(node, false, edit, Some(&snapshot), &|_| {})
                    .await;
                let _ = std::fs::remove_file(&snapshot);
                let item = written?;
                let wrote = place.is_some()
                    || self.remote.get(&node).is_none_or(|before| {
                        before.content_version() != item.content_version()
                            || before.modified != item.modified
                    });
                Ok(Done {
                    node,
                    content: item.content_version(),
                    modified: item.modified,
                    stat: seen,
                    here: None,
                    there: place.map(|p| (p.parent.unwrap_or(self.root), p.name)),
                    fresh: None,
                    uploaded: true,
                    wrote,
                })
            }
            Upload::Fresh(i) => {
                let entry = &self.fresh[i];
                let parent = self.node_of(entry.pos).ok_or(Error::UnknownParent)?;
                let source = self.path(Pos::Fresh(i)).ok_or(Error::UnknownParent)?;
                let new = NewItem {
                    request: NodeId::random().to_hex(),
                    place: Place {
                        parent: Some(parent),
                        name: entry.name.clone(),
                    },
                    folder: false,
                    may_exist: false,
                    created: entry.created,
                    modified: Some(millis(entry.stat.mtime)),
                };
                let snapshot = self.snapshot(&source, entry.stat).await?;
                let created = self.sync.writer.create(new, Some(&snapshot), &|_| {}).await;
                let _ = std::fs::remove_file(&snapshot);
                let item = created?.ok_or(Error::Malformed("no file created"))?;
                Ok(Done {
                    node: item.node,
                    content: item.content_version(),
                    modified: item.modified,
                    stat: entry.stat,
                    here: Some((entry.pos, entry.name.clone())),
                    there: Some((parent, entry.name.clone())),
                    fresh: Some(i),
                    uploaded: true,
                    wrote: true,
                })
            }
        }
    }

    /// A clone where the volume can make one, so an edit made during a long
    /// upload cannot mix into the version. Fails when the file changed
    /// since the scan.
    async fn snapshot(&self, source: &Path, seen: Stat) -> Result<PathBuf> {
        let copy = self.staging.file()?;
        let (from, to) = (source.to_owned(), copy.clone());
        let copied = blocking(move || {
            std::fs::copy(&from, &to)?;
            match scan::probe(&from)? {
                Some((false, now)) if now == seen => Ok(()),
                _ => Err(Error::SourceChanged),
            }
        })
        .await;
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&copy);
            return Err(e);
        }
        Ok(copy)
    }

    fn touches(&mut self, nodes: &[NodeId]) -> Result<()> {
        for &node in nodes {
            let (Some(item), Some(&seen), Some(base), Some(path)) = (
                self.remote.get(&node),
                self.found.get(&node),
                self.base.get(&node),
                self.path(Pos::Node(node)),
            ) else {
                continue;
            };
            let (modified, content) = (item.modified, base.content);
            match touch(&path, seen, modified) {
                Ok(Some(stat)) => {
                    self.report.applied += 1;
                    self.record(node, false, content, modified, stat)?;
                }
                Ok(None) => self.skip_node(node, Error::SourceChanged)?,
                Err(e) => self.skip_node(node, e.into())?,
            }
        }
        Ok(())
    }

    /// Records what both sides now agree on. The place is the one both
    /// show, else the one they last agreed on, so a move cut short is
    /// tried again rather than taken back.
    fn record(
        &mut self,
        node: NodeId,
        folder: bool,
        content: Option<[u8; 32]>,
        modified: Option<i64>,
        stat: Stat,
    ) -> Result<()> {
        let Some((parent, name)) = self.agreed_place(node) else {
            return Ok(());
        };
        let entry = Synced {
            node,
            parent,
            name,
            folder,
            content,
            modified,
            local: stat,
        };
        self.sync
            .store
            .record_synced(self.folder.id, std::slice::from_ref(&entry))?;
        self.base.insert(node, entry);
        Ok(())
    }

    /// A move, nothing else.
    fn record_place(&mut self, node: NodeId) -> Result<()> {
        let Some(base) = self.base.get(&node).cloned() else {
            return Ok(());
        };
        self.record(node, base.folder, base.content, base.modified, base.local)
    }

    fn agreed_place(&self, node: NodeId) -> Option<(NodeId, String)> {
        let local = self
            .here
            .get(&node)
            .and_then(|(pos, name)| Some((self.node_of(*pos)?, name.clone())));
        let remote = self.there.get(&node).cloned();
        match (local, remote, self.base.get(&node)) {
            (Some(local), Some(remote), _) if local == remote => Some(local),
            (_, _, Some(base)) => Some((base.parent, base.name.clone())),
            (local, remote, None) => local.or(remote),
        }
    }

    fn local_place(&self, node: NodeId) -> Option<Place> {
        let (pos, name) = self.here.get(&node)?;
        Some(Place {
            parent: Some(self.node_of(*pos)?),
            name: name.clone(),
        })
    }

    fn node_of(&self, pos: Pos) -> Option<NodeId> {
        match pos {
            Pos::Node(node) => Some(node),
            Pos::Fresh(i) => self.fresh[i].node,
        }
    }

    fn path(&self, pos: Pos) -> Option<PathBuf> {
        let relative = self.relative(pos)?;
        Some(if relative.is_empty() {
            self.folder.path.clone()
        } else {
            self.folder.path.join(relative)
        })
    }

    fn relative(&self, pos: Pos) -> Option<String> {
        let mut names = Vec::new();
        let mut current = pos;
        for _ in 0..=self.here.len() + self.fresh.len() {
            let (parent, name) = match current {
                Pos::Node(node) if node == self.root => {
                    names.reverse();
                    return Some(names.join("/"));
                }
                Pos::Node(node) => {
                    let (parent, name) = self.here.get(&node)?;
                    (*parent, name.as_str())
                }
                Pos::Fresh(i) => (self.fresh[i].pos, self.fresh[i].name.as_str()),
            };
            names.push(name);
            current = parent;
        }
        None
    }

    fn local_depth(&self, node: NodeId) -> usize {
        self.relative(Pos::Node(node))
            .map_or(0, |path| path.matches('/').count())
    }

    fn remote_depth(&self, node: NodeId) -> usize {
        let mut depth = 0;
        let mut current = node;
        while let Some((parent, _)) = self.there.get(&current)
            && depth <= self.there.len()
        {
            depth += 1;
            current = *parent;
        }
        depth
    }

    fn skip_node(&mut self, node: NodeId, error: Error) -> Result<()> {
        let path = self
            .relative(Pos::Node(node))
            .or_else(|| self.there.get(&node).map(|(_, name)| name.clone()))
            .unwrap_or_else(|| node.to_hex());
        self.skip(path, error)
    }

    fn skip_fresh(&mut self, i: usize, error: Error) -> Result<()> {
        let path = self
            .relative(Pos::Fresh(i))
            .unwrap_or_else(|| self.fresh[i].name.clone());
        self.skip(path, error)
    }

    /// Only the local database failing stops the pass.
    fn skip(&mut self, path: String, error: Error) -> Result<()> {
        if let Error::Database(_) = error {
            return Err(error);
        }
        self.report.failed.push((path, error));
        Ok(())
    }
}

/// `node` and what `children` puts under it.
fn subtree(node: NodeId, children: &HashMap<NodeId, Vec<NodeId>>) -> Vec<NodeId> {
    let mut seen = HashSet::from([node]);
    let mut nodes = vec![node];
    let mut at = 0;
    while at < nodes.len() {
        for kid in children.get(&nodes[at]).into_iter().flatten() {
            if seen.insert(*kid) {
                nodes.push(*kid);
            }
        }
        at += 1;
    }
    nodes
}
