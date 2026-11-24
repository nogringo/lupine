# Lupine architecture

Lupine is an end-to-end encrypted drive. File metadata lives in Nostr events, file content in Blossom blobs. Neither relays nor Blossom servers can read names, folder structure or content.

## Constraints

- Native apps for six platforms: macOS first, then iOS, Android, Windows, Linux and the web.
- Files up to 50 GB, in both directions.
- macOS: Finder integration through File Provider (files on demand), plus two-way sync of any folder the user picks.

## Rust core, native shells

Everything that must behave identically on every platform is written once, in Rust. Everything that touches the operating system is native.

| Layer | Location | Contents |
|---|---|---|
| Core | `crates/lupine-core` | Formats, crypto, chunking, transfers, Blossom and Nostr clients, local state, sync engine |
| Bindings | `crates/lupine-ffi` | UniFFI facade for Swift and Kotlin (WASM bindings for the web later) |
| Shells | `apple/`, later one per platform | UI, file system integration, keychain, file watching, background transfer scheduling |

Why: the format and the sync engine are where bugs lose data. Six implementations would mean six sources of corruption and a 6x6 interop matrix. Rust compiles without a runtime or GC to all six targets (WASM included), which matters inside File Provider extensions, and rust-nostr covers NIP-44, NIP-46, NIP-59 and negentropy (NIP-77).

The core does not own the network transport for blobs. On iOS a 50 GB upload must go through a background `URLSession` to survive suspension, so each platform can plug its own transport behind the `BlobStore` trait.

`docs/protocol.md` specifies every format, so a native reimplementation stays possible for any platform that needs one.

### Platform integrations

| Platform | Shell | File system integration |
|---|---|---|
| macOS | SwiftUI menu bar app | `NSFileProviderReplicatedExtension`, folder sync with FSEvents |
| iOS | SwiftUI | File Provider extension, background `URLSession` |
| Android | Kotlin, Compose | `DocumentsProvider`, WorkManager |
| Windows | to decide | Cloud Files API (placeholders) |
| Linux | to decide | Folder sync |
| Web | TypeScript UI, core in WASM | Streams, File System Access API |

## Identity and keys

```
user identity (nsec, NIP-46 bunker, NIP-55, NIP-07)
  └─ drive descriptor event, NIP-44 encrypted to self
       └─ drive secret (a secp256k1 key: the drive pubkey)
            ├─ signs Blossom auth tokens and deletions
            └─ metadata key ── with each wrap's pubkey, encrypts and authenticates every node version
                                 └─ per-file key ── encrypts that file's chunks
```

The user's identity is used once per device, to read the drive descriptor. After that, the drive's own keys do everything:

- A NIP-46 bunker cannot sign the 6,000 Blossom tokens of a 50 GB upload, or decrypt thousands of versions during an initial sync.
- Relays and Blossom servers see an anonymous pubkey, not the user's npub. Only the descriptor event, whose content is encrypted, links the two.

Cost: paid Blossom servers and relays requiring auth must allow the drive pubkey rather than the user's npub, and again after each key rotation. The app has to show it, and does in its settings.

Signing in takes an nsec, a `bunker://` link, or a `nostrconnect://` pairing that a NIP-46 signer scans as a QR code. The device finds the user's write relays in their latest kind 10002, on index relays, reads the descriptors there and opens one, or creates a drive. It then forgets the user's key: the keychain holds the drive secret, and the local database the descriptor event, which the device republishes to the relays that lack it without needing the user's key. A new drive starts from the user's own inbox relays (kind 10050) and Blossom servers (kind 10063), which the user edits. Before announcing it, the device checks with BUD-06 (`HEAD /upload`) that each server takes an encrypted chunk, which media hosts refuse.

Losing the drive secret loses the drive, and leaking it gives the drive away. Every device republishes the descriptor where it is missing, and the user can export a recovery key. After a device is lost, a key rotation replaces the drive key: it locks out a device that can no longer decrypt with the user's key, such as a revoked NIP-46 session, and the recovery keys exported before. Every other device needs the user's key, or the new recovery key, once more.

## Data model

- A node is a file or a folder with a stable random id, and it points to its parent. Renaming or moving a folder publishes one new version, whatever the folder holds.
- Every change publishes a new version, never replaced. Its version vector records which versions it replaces, so a device recognizes an old version even after the ones in between were deleted. Addressable events would be simpler, but a relay keeps only the latest one per node. Concurrent edits and edits delivered late would be silently dropped, relays could diverge, history would be lost, and the `d` tag would let relays link every version of a node. The append-only lists NIP (kinds 1990 and 1991) moved NIP-51 lists away from replaceable events for the same reasons.
- A version travels in an event shaped like a NIP-59 gift wrap addressed to the drive pubkey: kind 1059, a throwaway signing key, a random timestamp, a NIP-44 payload. Inside there is no seal, only the version encrypted with a key derived from the drive's metadata key and the wrap's signing key: it proves the version comes from the drive and ties it to that wrap. Versions are padded to the size of a one-word DM. To relays, a drive looks like an inbox of direct messages: there is no Lupine kind to query, and relays that follow NIP-17 serve the wraps only to the drive.
- A version is identified by the hash of its content, so the same version in two wraps (a retried publication, a key rotation) counts once.
- File content is cut into fixed-size chunks (8 MiB by default), each encrypted into its own blob as 64 KiB segments. Small files list their chunks inline; large files store the chunk table in a separate encrypted blob, which keeps events small.
- Deleting a node publishes a tombstone, so other devices learn about it. Deleting a folder publishes one per node it holds, as creating them published one version each. Tombstones expire after a year.
- Versions superseded for longer than the history window are deleted with NIP-09. This sets how far back history goes and keeps growth small: relays keep the deletion requests, about 70 bytes per deleted version. Blobs no version has referenced for 30 days are collected afterwards.

## Large files

Blossom has no chunked or resumable upload (BUD-00 to BUD-13), so chunking happens on the client. A file is cut into fixed-size chunks, one blob each, and each chunk is encrypted as 64 KiB segments that carry their own tag. This gives:

- resumable and parallel transfers, chunk by chunk. Encryption is deterministic, so a chunk re-encrypted after a crash is the same blob;
- random access: a read fetches only the segments it needs, with HTTP `Range` (BUD-01). Seeking in a 50 GB video costs 64 KiB, not a whole chunk, and File Provider partial fetches map to segments;
- delta re-upload: chunks whose plaintext did not change keep their blob;
- bounded memory: one chunk per transfer slot, encrypted in place.

Measured on this Mac, encrypting a 20 GiB file: 2.2 GB/s with 4 chunks in flight for 36 MB of memory, 3 GB/s with 8 for 70 MB. The network is the limit, not the CPU.

The 8 MiB chunk size is a trade-off, not a measured optimum. It means about 6,000 requests for 50 GB, one 8 MiB buffer per transfer slot (which matters inside iOS extensions), and 8 MiB re-uploaded for a one-byte change. Segments removed the main cost of large chunks, the time to the first byte after a seek. The chunk size is recorded per file, so it can change later.

Server size limits are not a constraint. Probed in November 2026 with `HEAD /upload`: nostr.build and blossom.band cap uploads at 100 MiB, nostrcheck below that, nostr.download accepts at least 100 MiB, and the reference blossom-server defaults to 2 GB.

Server policies are one. nostr.build, blossom.band and nostrcheck only host media and refuse `application/octet-stream`, so they cannot store encrypted chunks. A drive needs servers that store arbitrary blobs: nostr.download accepted them, and a self-hosted blossom-server does.

The device uploads each chunk once, to the first server of the drive's list. Servers that implement BUD-13 mirroring (`POST /<sha256>`) then fetch it from there, and the others get their own upload. In November 2026 few servers mirror: nostr.download accepted BUD-13 uploads but did not advertise mirroring, and the reference blossom-server implemented neither, so most servers still cost the device one upload each.

The client signs one BUD-11 token per request, scoped to one server and one blob: the drive key signs locally in microseconds, and a leaked token can only repeat that request. A token may carry many `x` tags to authorize a batch, but HTTP header size caps a batch at a few dozen hashes.

Reads go to the servers in order and move on when one fails, lacks the blob or serves a whole blob that does not match its hash.

## Processes on macOS

- `Lupine.app`, a menu bar agent started at login. It keeps the relay subscriptions, runs folder sync and signals the File Provider enumerators when the remote tree changes. The extension cannot do this itself: the system stops it when idle, and Nostr has no push.
- `LupineFileProvider.appex` serves Finder: enumerates from the local database, fetches (partial) contents from Blossom, uploads new and modified files and writes Finder's changes to the local database. It does not reach the relays: it leaves the new wraps in an outbox and posts a Darwin notification, on which the app sends them.
- Both share an SQLite database in the App Group container (`936HBZB935.ovh.uid.lupine`) and keys in a shared keychain group.
- The app opens a window to sign in when no drive is set up, and registers the Finder location only once one is. Signing out stops both syncs, removes the Finder location (files Finder had not handed over are set aside and shown), then the database and the drive secret. Synced folders keep their files.
- The extension fetches content into the system's temporary directory. A partial fetch writes the requested range, widened to whole 64 KiB segments and to the system's alignment, at its offsets in a sparse file of the full size.
- The extension uploads from an APFS clone of the file the system hands it, so an edit made during the upload cannot mix into the version. An edit uploads only the chunks that changed, under the file's key.
- Finder has no trash for the drive: it asks, then deletes for good. `.DS_Store` files, symbolic links and aliases stay on the Mac.

Both targets are sandboxed. Folder sync uses security-scoped bookmarks, so distribution through Developer ID or the Mac App Store both stay possible.

## Sync

- The set of versions is the remote truth, and the union of what every relay holds converges. The local database mirrors it: negentropy (NIP-77) reconciles sets, so a version delivered late is picked up whatever its `created_at`, and a live subscription brings new ones.
- A sync reconciles two sets with each drive relay: the wraps addressed to the drive and the drive's deletions, deletions first. Only ids travel until the missing events are fetched. Relays without NIP-77 are listed with paged REQs instead. Relays that follow NIP-17 serve the wraps after NIP-42 AUTH, which the drive key answers. The sync then sends each relay the wraps it lacks and the deletions of wraps it still holds, so relays converge without a server-side mirror.
- Each sync also reads the drive's lists (kinds 10050 and 10063) on its relays, and applies the ones another device changed: the app then opens the drive again with them. When the servers change, the device copies every blob it knows of to the servers that lack it, after a full sync so that it knows every version, and keeps reading from the servers the drive left until the copy is done. A fill cut short or failed starts over an hour later, or at the next launch: it only copies what is missing.
- A session compacts once it has synced every drive relay, then once a day while it runs: it deletes the versions superseded for longer than the history window, the tombstones past their lifetime and the junk sent to the drive. The history window is a setting of each device, from a week to a year, so the shortest one among a drive's devices applies.
- Once a day, after a full sync, the app collects blobs: it deletes from every drive server those that no version it holds has referenced for 30 days, and those it uploaded for a version that never came. The 30 days count from the first collection that found a blob unreferenced, or from its upload, so a device that edits a file from an older version can still reuse its chunks. One that finds a chunk collected anyway uploads the file again, as the same blobs. A device that holds a version it cannot read, such as a later format, deletes nothing, since that version may reference any blob.
- A key rotation moves the drive to a new key, as the protocol describes. The rotating device signs the new descriptor with the user's key first, then needs only the drive keys: it builds the new key's wraps in a scratch database next to the local one, sends them, publishes the descriptor and the old key's request to vanish, and moves the local database to the new key in one transaction. Items, synced folders and device ids stay, so Finder and folder sync see no change, and every step resumes after a crash. Syncing stops meanwhile, and the old key's wraps are deleted afterwards, from the next sync on.
- Other devices stop at the old key's request to vanish, before they apply its deletions, and ask the user to sign in again. They then sync the new key into a scratch database and move the same way, wrapping again the changes they had not sent.
- Version vectors tell which versions of a node are concurrent. Several heads mean concurrent edits: every device shows the same winner, an edit wins over a deletion, and any device resolves them with a version that replaces every head, differing contents becoming conflict copies. Devices resolve conflicts and publish tree repairs only after a full sync, since the version that settles things may only be late.
- Deterministic rules settle the tree, so every device shows the same one: a folder deleted while something moved into it comes back, cycles from concurrent moves are broken, and names that collide, case and Unicode normalization included, get a suffix.
- A synced folder is a pair (local path, drive folder), reconciled by a three-way merge of the local scan, the last synced state and the drive folder. See [Folder sync](#folder-sync).
- A change from Finder replaces the heads that hold the state Finder showed, and leaves any other head in conflict. New content made from content Finder no longer shows replaces nothing: it stays in conflict with what came since, which becomes a conflict copy. Deleting a file whose content changed since Finder showed it keeps the file and shows it again; an edit of a node deleted elsewhere brings it back.

## Local database

Each drive has its own SQLite database, in WAL mode, shared by the app and the File Provider extension. The app writes what it syncs, the extension what Finder changes.

- It holds what the device knows of the drive: every wrap and deletion seen, the versions they carry, and the events relays may still need. A launch restores the replica from it without decrypting or verifying anything again, and the next sync downloads only what is new.
- It also holds the tree as Finder shows it, one row per item. Each change to an item takes the next number of a counter, which is the File Provider sync anchor. Anchors carry a random id of the database, so an anchor from a deleted and recreated database expires instead of skipping changes. Items that leave the tree stay as removed rows, so a change enumeration reports them.
- The app writes it after every step (a sync, a batch of live events, a publication) and signals the working set when items changed. The working set is the whole drive, since the extension does not track which folders are materialized yet.
- The extension writes each version with the item Finder asked for, in one transaction, so Finder finds the item at once. When the tree rules show something else, such as a suffix for a colliding name, the app corrects the item once it takes the version in.
- The wraps of the extension and folder sync wait in an outbox, in the order written. The app takes them into its replica and sends them a wave at a time: a folder's tombstone leaves only once those of what it held reached a relay, so a deletion cut short never revives a folder. Sync repairs leave the outbox alone.
- It holds decrypted metadata, file keys included, like the files a device downloads. The drive secret stays in the keychain.
- It holds every blob the device knows of, with since when no version it holds references it, and the blobs each chunk table lists, so that a collection reads only the tables it never read. The extension and folder sync note each blob before uploading it, and mark it referenced with the version that uses it, in the same transaction.
- It holds the descriptor this device signed in with, the drive's name and the user's write relays, so that the app republishes the descriptor at each launch. A drive opened with a recovery key has none. It also holds the history window this device compacts with, when it applied the drive's lists, and the servers the drive is leaving until a fill completes.
- It holds the key rotation this device started, with the descriptor the user signed, until it finishes, then the retired key's request to vanish and the wraps it still has to delete. A writer of the extension or folder sync whose key is no longer the database's fails with a transient error, and opens the drive again. The keychain holds the new secret during a rotation, and the old one until its wraps are deleted.
- It holds three device ids per drive: the session's, the extension's and folder sync's. Version vectors take a device's versions to follow one another, each written after seeing the last, which two writers at once would break. The database is excluded from backups: a restored copy would reuse these ids. Losing the database costs a full sync, and Finder changes not yet sent.
- For each synced folder, it holds what every entry was when both sides last agreed: its node, its place, the content version of the drive's file, and the file id, size and modification time of the local one.

## Folder sync

A synced folder pairs a folder of the device with a drive folder: the one of the same name at the drive root, made when there is none. Synced folders may not nest. The app watches each one with FSEvents, ignoring its own changes, and runs a pass when it changes, when the drive changes, and every 5 minutes. Passes wait until the session has synced every drive relay once: until then the drive folder may lack entries that a pass would take for deleted.

- A pass merges three trees: the folder as scanned, the drive folder as the local database shows it, and what both were when they last agreed. What changed on one side is applied to the other.
- The scan tells a move from a deletion by file id. It looks for each entry in place, then under the same file id elsewhere, then in place under a new file id, as a save that writes a new file leaves it. Size, modification time and file id tell a file changed. Renaming a folder publishes one version, whatever it holds. A move made while the scan runs would look like a deletion, so a pass that misses an entry scans again.
- What changed on both sides is settled as the drive settles concurrent versions. An edit wins over a deletion, on either side. The drive's place wins over a local move. Two contents become concurrent versions, so the conflict copy reaches both sides. Equal contents are no conflict: uploaded with the drive file's key, every chunk is found unchanged and nothing is written.
- Entries new on both sides at the same place pair up, by name as a case-insensitive volume compares them, so adding a folder the drive already holds merges the two.
- A folder deleted on one side is deleted on the other only when nothing in it changed or arrived since. Otherwise it comes back, with what changed. A folder moved or renamed on this device while the drive deleted it comes back with all it holds. A deletion on the drive checks, as its tombstones are written, that it takes only what the pass decided to delete.
- What the drive deletes goes to the Trash, where it can be put back.
- Each change checks the state it was decided on, and leaves the entry to the next pass when that state is gone. A download replaces only the file the scan saw (`renamex_np` with `RENAME_SWAP`, then a check of what it swapped out), a rename never replaces anything (`RENAME_EXCL`), and the Trash takes only what both sides agreed on. Two moves that would make a loop, one on each side, settle with the drive's place. The record of an entry is written after each change: a pass cut short in between finds both sides equal and records them.
- Downloads and upload snapshots go through `.lupine-sync`, a staging folder inside the synced folder, so they reach their place with a rename on the same volume. Uploads read an APFS clone, so an edit made during a long upload cannot mix into the version. Modification and creation times travel with the content.
- A pass does nothing when the folder is missing, when another folder is at its path (the folder's file id is recorded), or when the drive folder was deleted. What the scan cannot read, and what the drive cannot hold, is left alone, and folders that hold it never go to the Trash.
- Never synced: `.DS_Store`, `Icon\r`, symbolic links, other volumes, and names the drive cannot hold. The name `.lupine-sync` is reserved at the top of a synced folder.
- Folder sync writes versions under a device id of its own and leaves their wraps in the outbox, as the File Provider extension does: it runs beside the session, which writes repairs of its own.

## Open questions

- Event kind 3540 (descriptor) is provisional, pending a published spec. It is absent from the NIPs registry and was unused on the relays checked in November 2026.
- Relay behavior at scale: how inbox relays treat 100,000 gift wraps to one pubkey, published in bursts by throwaway keys (quotas, anti-spam, retention). To test. A drive should live on relays the user controls or pays for.
- History window: how long superseded versions stay before compaction. The default of 30 days, the tombstone lifetime (365 days) and the blob grace period (30 days) are first guesses. Since each device applies its own window, one set short shortens the drive's history for all.
- Blob ownership: Blossom does not define it, so what a rotation can do about the old key's blobs depends on each server. Servers that do not mirror keep them under the old key only.
- Concurrent rotations: two devices that rotate at once publish two descriptors of the same epoch. The latest wins, and the other device stays on its own key.
- Retired keys: a device still on the old key relies on the request to vanish staying beside the old key's deletions. A relay that serves it during the retirement, then drops it and keeps the deletions, would bring them to such a device. Deferring every drive deletion that names a live head, which compaction never does, would keep the tree whatever relays keep.
- Adopting a rotation syncs every drive relay: one that is down for good blocks it until a device on the new key removes it from the drive.
- Servers that refuse deletions (BUD-12) keep every blob, and each collection asks them again.
- Blossom servers: which general-purpose servers to recommend or run, since media hosts refuse encrypted blobs. Range support also needs checking server by server.
- Sharing: planned as "spaces", folders with their own key and member list, shared through NIP-59 gift wraps. A shared key lets every member write as any other, so members will sign their versions inside the payload. Not designed yet.
- UI toolkits for Windows and Linux.
- Finder deletions: deleting a folder also deletes what reached the database under it after Finder last enumerated it. The extension could keep those, as it keeps a file edited since.
- Tags, extended attributes and Finder flags are not kept yet.
- Folder sync scans the whole folder at each pass. FSEvents could tell which folders to scan again.
- Mass deletions: emptying a synced folder empties its drive folder, as two-way sync means. Other clients ask before they pass one on.
- Folder sync on Windows and Linux: without file ids in the scan, a move shows as a deletion and a creation.

## Roadmap

1. Content format and transfer pipeline, macOS skeleton (done).
2. Blossom client: BUD-01, 02, 11, 12 and 13, mirroring to every server (done).
3. Nostr: drive descriptor, node versions and their graph, compaction, relay sync, blob collection, key rotation (done).
4. Local database and the File Provider read path (enumerate, fetch, partial fetch) (done).
5. File Provider write path (create, modify, move, delete) (done).
6. Folder sync engine (done).
7. Sign-in (nsec, NIP-46), onboarding, settings (done).
