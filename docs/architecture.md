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

Cost: paid Blossom servers and relays requiring auth must allow the drive pubkey rather than the user's npub, and again after each key rotation. The app has to show it.

Losing the drive secret loses the drive, and leaking it gives the drive away. Every device republishes the descriptor where it is missing, and the user can export a recovery key. After a device is lost, a key rotation replaces the drive key: it locks out a device that can no longer decrypt with the user's key, such as a revoked NIP-46 session, and every other device needs the user's key once more.

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
- `LupineFileProvider.appex` serves Finder: enumerates from the local database, fetches (partial) contents from Blossom, uploads new and modified files.
- Both share an SQLite database in the App Group container (`936HBZB935.ovh.uid.lupine`) and keys in a shared keychain group.

Both targets are sandboxed. Folder sync uses security-scoped bookmarks, so distribution through Developer ID or the Mac App Store both stay possible.

## Sync

- The set of versions is the remote truth, and the union of what every relay holds converges. The local database mirrors it: negentropy (NIP-77) reconciles sets, so a version delivered late is picked up whatever its `created_at`, and a live subscription brings new ones.
- A sync reconciles two sets with each drive relay: the wraps addressed to the drive and the drive's deletions, deletions first. Only ids travel until the missing events are fetched. Relays without NIP-77 are listed with paged REQs instead. Relays that follow NIP-17 serve the wraps after NIP-42 AUTH, which the drive key answers. The sync then sends each relay the wraps it lacks and the deletions of wraps it still holds, so relays converge without a server-side mirror.
- Version vectors tell which versions of a node are concurrent. Several heads mean concurrent edits: every device shows the same winner, an edit wins over a deletion, and any device resolves them with a version that replaces every head, differing contents becoming conflict copies. Devices resolve conflicts and publish tree repairs only after a full sync, since the version that settles things may only be late.
- Deterministic rules settle the tree, so every device shows the same one: a folder deleted while something moved into it comes back, cycles from concurrent moves are broken, and names that collide, case and Unicode normalization included, get a suffix.
- A synced folder is a pair (local path, remote folder node), reconciled by a three-way merge of the local scan, the last synced state and the remote tree.
- Files are snapshotted with APFS `clonefile` before upload, so editing a file during a long upload cannot produce a mixed version.

## Open questions

- Event kind 3540 (descriptor) is provisional, pending a published spec. It is absent from the NIPs registry and was unused on the relays checked in November 2026.
- Relay behavior at scale: how inbox relays treat 100,000 gift wraps to one pubkey, published in bursts by throwaway keys (quotas, anti-spam, retention). To test. A drive should live on relays the user controls or pays for.
- History window: how long superseded versions stay before compaction. The default of 30 days, the tombstone lifetime (365 days) and the blob grace period (30 days) are first guesses.
- Blob ownership: Blossom does not define it, so what a rotation can do about the old key's blobs depends on each server.
- Blossom servers: which general-purpose servers to recommend or run, since media hosts refuse encrypted blobs. Range support also needs checking server by server.
- Sharing: planned as "spaces", folders with their own key and member list, shared through NIP-59 gift wraps. A shared key lets every member write as any other, so members will sign their versions inside the payload. Not designed yet.
- UI toolkits for Windows and Linux.

## Roadmap

1. Content format and transfer pipeline, macOS skeleton (done).
2. Blossom client: BUD-01, 02, 11, 12 and 13, mirroring to every server (done).
3. Nostr: drive descriptor, node versions and their graph, compaction, relay sync (done; blob collection and key rotation remain).
4. Local database and the File Provider read path (enumerate, fetch, partial fetch).
5. File Provider write path (create, modify, move, delete).
6. Folder sync engine.
7. Sign-in (nsec, NIP-46), onboarding, settings.
