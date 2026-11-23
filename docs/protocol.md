# Lupine protocol, v1 (draft)

Event kind 3540 is provisional. `crates/lupine-core` implements everything below except blob collection and key rotation.

## Conventions

- Hex is lowercase. Base64 is RFC 4648 standard alphabet with padding.
- Integers are big-endian. `||` is concatenation.
- `HKDF(ikm, info)` is HKDF-SHA256 with no salt and a 32-byte output.
- `HMAC(key, data)` is HMAC-SHA256.
- `padme(n)` is the Padmé function (Nikitin et al., PETS 2019); `padme(n) = n` for `n < 2`.
- `NIP44(key, plaintext)` is the base64 NIP-44 v2 payload of `plaintext`, computed with `key` as the conversation key and a random nonce.

## Envelope

External chunk tables use this format:

```
envelope = 0x01 || nonce (24 bytes) || ciphertext || tag (16 bytes)
```

The cipher is XChaCha20-Poly1305 (libsodium's `crypto_aead_xchacha20poly1305_ietf`). Each use below defines the key, the nonce and the associated data (AAD). Readers reject any first byte other than `0x01`.

## Keys

| Key | Definition |
|---|---|
| `drive_secret` | 32 bytes, the secp256k1 secret key of the drive pubkey |
| `metadata_key` | `HKDF(drive_secret, "lupine/v1/drive/metadata")` |
| `wrap_key` | `HKDF(metadata_key, "lupine/v1/wrap" \|\| pubkey)`, one per gift wrap, where `pubkey` is the wrap's 32-byte signing pubkey |
| `file_root` | 32 random bytes per file, stored in its versions |
| `file_enc` | `HKDF(file_root, "lupine/v1/file/enc")` |
| `file_mac` | `HKDF(file_root, "lupine/v1/file/mac")` |
| `file_nonce` | `HKDF(file_root, "lupine/v1/file/nonce")` |
| `file_table` | `HKDF(file_root, "lupine/v1/file/table")` |

## Drive descriptor: kind 3540

Regular event signed by the user's identity key. One is written when the drive is created, renamed or rotated.

- No tags.
- `content`: NIP-44 v2 encryption, to the author's own pubkey, of:

```json
{
  "v": 1,
  "id": "<16 random bytes, hex>",
  "epoch": 0,
  "secret": "<drive_secret, hex>",
  "name": "Lupine"
}
```

`id` identifies the drive for its whole life. `epoch` counts its key rotations, from 0: the drive pubkey changes with each epoch.

Among descriptors with the same `id`, the current one has the highest `epoch`, then the latest `created_at`. The client that publishes a new descriptor deletes the older ones with NIP-09.

Descriptors go to the user's write relays (NIP-65) and to the drive relays. A device keeps the descriptor event it read and republishes it, unchanged, to any of these relays that lacks it. The descriptor is lost only if every relay and every device loses it.

Deleting a drive ends with a NIP-09 deletion (kind 5) of its descriptors.

### Recovery key

Clients offer to export a recovery key: the bech32 encoding, with the prefix `lupine`, of a TLV list built as in NIP-19 (one byte of type, one byte of length, then the value):

| Type | Value |
|---|---|
| 0 | `drive_secret`, 32 bytes |
| 1 | `id`, 16 bytes |
| 2 | `epoch`, 4 bytes |
| 3 | A drive relay URL, ASCII. Repeated for each relay |

Importing it restores the drive without any descriptor: the client reads the versions from the relays it lists, then publishes a descriptor if none carries this `id` and `epoch`. A recovery key gives full access to the drive, like a device. A rotation makes the recovery keys exported before it useless, so clients then offer a new one.

### Key rotation

Rotation replaces the drive key, for example after a device is lost. The rotating client:

1. generates a new `drive_secret`, and publishes its kinds 10050 and 10063 with the same relays and servers;
2. asks every server to keep each referenced blob for the new key, with a mirror request (BUD-13 `POST /<sha256>`) whose source is that server's own URL for the blob, so no bytes leave the device;
3. wraps every version it keeps again, for the new pubkey. The JSON does not change, so version ids and `epoch` fields stay as they were;
4. publishes a descriptor with the same `id`, the next `epoch` and the new secret, then deletes the older descriptors;
5. deletes, with the old key, every wrap addressed to the old pubkey.

Other devices switch to the new key when they read the new descriptor, which takes the user's key again. A version never reuses the `file_root` of a version it replaces from an earlier epoch: the first edit of each file after a rotation uploads it whole, under a new key.

Rotation locks out a device that can no longer decrypt with the user's key, such as one whose NIP-46 session was revoked. A device that holds the user's nsec reads the new descriptor like the others. Content written before the rotation stays readable to the old key, which could have downloaded it already. Blossom does not define who owns a blob, so whether the old key can still delete blobs, and whether the new one can, depends on each server. Paid relays and servers must allow the new pubkey.

## Drive relays and servers

The drive key publishes them with existing kinds:

- kind 10050 (NIP-17): the relays that store the drive's versions. NIP-17 asks relays to serve gift wraps only to their `p`-tagged recipient, after NIP-42 AUTH, so only the drive can download its versions.
- kind 10063 (BUD-03): the Blossom servers. Every blob is stored on all of them.

Both go to the drive relays and to the user's write relays (NIP-65), where a new device finds the descriptor.

## Node versions

Every change to a node (creation, edit, rename, move, deletion) publishes a new version, which is never replaced. A version travels in an event shaped like a NIP-59 gift wrap:

- `kind`: 1059.
- `pubkey`: a fresh random key for every event, which signs it.
- `tags`: exactly `[["p", <drive pubkey>]]`.
- `created_at`: a random time within the last two days, as NIP-17 recommends.
- `content`: `NIP44(wrap_key, plaintext)`, with the `wrap_key` of this event's `pubkey`.

The plaintext is the version JSON followed by spaces (0x20) up to 896 bytes, which JSON allows. NIP-44 pads the seal of the shortest NIP-17 message to 896 bytes, so no version is smaller than a one-word DM. The plaintext, padding included, must fit in 65,535 bytes, the NIP-44 limit.

There is no seal or rumor inside. Anyone can publish a gift wrap to the drive pubkey, but only holders of `metadata_key` can produce a payload that passes the NIP-44 MAC. Readers verify the event's signature and ignore every wrap that fails to decrypt. Since `wrap_key` depends on the signing key, a payload decrypts only in the wrap it was made for. Nobody can move it into another wrap without the drive keys, so every copy a relay can hold is a wrap that a deletion can name.

A version's id is the SHA-256 of its decrypted plaintext, padding included. The same version can travel in several wraps, when a device retries a publication or a rotation wraps it again. Readers deduplicate versions by id.

### Version JSON

| Field | Type | Present | Meaning |
|---|---|---|---|
| `v` | number | always | `1` |
| `node` | hex | always | Node id: 16 random bytes, fixed for the life of the node |
| `type` | string | always | `file`, `folder` or `deleted` |
| `epoch` | number | always | Drive epoch the version was written in |
| `time` | number | always | When the version was written, in Unix milliseconds |
| `device` | hex | always | Id of the device that wrote the version |
| `vv` | object | always | Version vector, see below |
| `parent` | hex | optional | Parent node id. Absent at the drive root |
| `name` | string | always | 1 to 255 UTF-8 bytes in NFC, not `.` or `..`, no `/` or NUL |
| `created`, `modified` | number | optional | File times, in Unix milliseconds |
| `file` | object | file only | See below |

A node keeps its type until it is deleted. A `deleted` version is a tombstone: it keeps the `parent` and `name` the node had, so the node can be restored, and has no `file`. Readers ignore unknown fields.

A device id is 16 random bytes, generated once per install. Two installs never share one: an install cloned or restored from a backup generates a new id. Each part of an install that writes versions on its own, such as an app, its File Provider extension and its folder sync, has an id of its own: a device writes each version after seeing its last one.

Each device numbers the versions it writes, across all nodes. A new version takes the greater of the previous number plus one and the current Unix time in milliseconds, so a device that lost its counter does not reuse a number.

`vv` maps device ids to sequence numbers. Its entry for `device` is the version's own number. Every other entry is the highest number of that device among the versions this one replaces, directly or not.

Writers normalize names to NFC. Readers normalize them too, so a name written otherwise still compares right. Clients must map names to their local file system safely. For example, Windows also forbids `\`, `:` and a few reserved names.

### File object

| Field | Type | Meaning |
|---|---|---|
| `key` | base64 | `file_root` |
| `size` | number | Plaintext size in bytes |
| `chunk_size` | number | Power of two, from 65,536 to 67,108,864 |
| `chunks` | base64 | Inline chunk table, when it has at most 64 entries |
| `table` | hex | Blob address of the chunk table otherwise |

Exactly one of `chunks` and `table` is present.

### Heads and conflicts

`seq(V)` is `V.vv[V.device]`, and a missing entry counts as 0.

- Version V dominates version W of the same node when V's writer had seen W and W's writer had not seen V: `V.vv[W.device] >= seq(W)` and `W.vv[V.device] < seq(V)`. Dominance does not need the versions in between, so it still holds after compaction.
- A head is a version that no other version of the node dominates. A version that arrives late, or comes back after compaction, changes nothing when a head dominates it.
- A new version replaces the heads its writer knows, usually one. Its `vv` merges theirs, keeping the highest number for each device, then adds its own number under `device`.
- One head: it is the state of the node.
- Several heads: concurrent edits. The winning head is the most recent live head (greatest `time`, then lowest version id), or the most recent tombstone when every head is one. Every client shows the winner, so all devices agree, and an edit wins over a concurrent deletion.
- Heads that differ only in `epoch`, `time`, `device` and `vv` are not a conflict: the next version replaces them all.
- Otherwise a client resolves the conflict, after a full sync of every drive relay, with a version that replaces every head and keeps the winner's state. Each live head whose `file` differs from the winner's becomes a conflict copy, a new file next to the winner:
  - node id: the first 16 bytes of `SHA-256("lupine/v1/conflict" || version id)`, with the id of that head;
  - `name`: the head's name with ` (conflict YYYY-MM-DD)` before its extension, the date of the head's `time` in UTC;
  - `parent`: the winner's; `file`: the head's.

Two devices that resolve the same conflict create the same copies, and their versions differ only in `time`, `device` and `vv`.

### Tree

A node's parent and name are those of its winning head. Clients then apply these rules, in order, so they all show the same tree:

1. Deleted parents. A live node whose parent's winning head is a tombstone revives that parent as a folder, with the name and parent its tombstone kept, and so on up the tree. An edit or a move into a folder deleted at the same time thus wins over the deletion.
2. Missing parents. A node whose parent is unknown, or is not a folder, waits: the parent may only be late. Once the client has synced every drive relay, it shows the node at the drive root.
3. Cycles. When parents form a loop, the node of the loop whose winning head is the most recent (greatest `time`, then lowest version id) goes to the drive root.
4. Names. Two live nodes of a folder collide when their names are equal after NFC normalization and Unicode default case folding, so that case-insensitive file systems can hold the tree. The node with the lowest id keeps its name. The others, in order of node id, take the first free name among `<stem> (2)<ext>`, `<stem> (3)<ext>` and so on.

The extension of a name starts at its last `.`, unless that `.` is the first character. When a suffix makes a name longer than 255 bytes, the stem is cut at a character boundary. An extension too long to leave room for the suffix counts as part of the stem.

Rules 1, 3 and 4 show a state that no version holds. After a full sync of every drive relay, the first client that applies one publishes it: a folder version that dominates the tombstone, a move to the root, a rename. Clients that publish the same repair at the same time produce versions that differ only in `time`, `device` and `vv`. Nodes that rule 2 shows at the root are not repaired: their parent may still arrive, so they keep their parent and name. A node that needs a repair and a conflict resolution gets one version that does both.

Deleting a folder publishes a tombstone for every node under it, deepest first, and the folder's own last, so a deletion cut short never revives a folder.

### Compaction

A superseded version stays for the history window, a client setting. Once a version that dominates it is older than the window, by `time`, the drive key deletes every wrap carrying it. A node whose heads are all tombstones, the latest older than the tombstone lifetime of 365 days, loses its tombstones too.

Deletions are NIP-09 kind 5 events signed by the drive key:

- one `e` tag per wrap id, and a `["k", "1059"]` tag;
- split to stay under relay size limits.

NIP-59 has relays delete the `kind:1059` events whose `p` tag matches the signer of a deletion, so the drive, as recipient, deletes its own wraps. Deletions name wrap ids: version ids are never published, so a relay could not match them. Relays keep deletion requests, as NIP-09 asks: about 70 bytes per deleted wrap, against 1.7 KB for a small wrap.

A device that comes back within the tombstone lifetime learns about every deletion from the tombstones. A node the client knew that has no version left on the drive relays was deleted before that.

### Deletions and repair

- Clients ignore every wrap that a drive deletion names.
- A client looks for a drive deletion, with `{"kinds": [5], "authors": [<drive pubkey>], "#e": [<wrap ids>]}`, before it trusts a wrap that some drive relays lack, or a version older than the tombstone lifetime of a node it does not know. Without this, a relay that missed a deletion could bring back a node deleted more than a year ago. For the nodes a client knows, version vectors already set old versions aside.
- When drive relays disagree on a wrap, the client sends the deletion that names it to the relays that still hold it, or, when there is none, the wrap to the relays that lack it.
- Anyone can send gift wraps to the drive pubkey, which is public. The drive deletes the ones that fail to decrypt like its own wraps, and clients skip their ids until the relays have deleted them. A wrap that decrypts but holds a version the client cannot read, such as a later format, is kept and ignored.

## File content

A file is cut into chunks, one blob each. A chunk is encrypted as a run of 64 KiB segments that each carry their own tag, so a reader can fetch and verify any part of a blob on its own.

### Chunks

- Chunk `i` covers bytes `[i * chunk_size, min((i + 1) * chunk_size, size))`. An empty file has no chunks.
- The plaintext of each chunk is its bytes. The last chunk is zero-padded to `min(chunk_size, max(4096, padme(length)))`.
- `mac_i = HMAC(file_mac, i (u64) || unpadded plaintext of chunk i)`. A client re-uploading a file with the same key and chunk size keeps the blob of every chunk whose MAC did not change. Keys are not reused across a key rotation.

### Segments

- The padded plaintext is split into segments of 65,536 bytes; the last one may be shorter.
- `prefix_i` is the first 19 bytes of `HMAC(file_nonce, mac_i)`.
- Segment `j` is encrypted with XChaCha20-Poly1305, key `file_enc`, `nonce = prefix_i || j (u32) || last` where `last` is `0x01` for the blob's last segment and `0x00` otherwise, and `AAD = "lupine/v1/segment" || i (u64)`.
- `blob_i` is, for each segment in order, its ciphertext followed by its 16-byte tag. There is no header: segment `j` starts at byte `j * 65,552`. The Blossom address is `SHA-256(blob_i)`.

Encryption is deterministic: the same plaintext at the same position gives the same blob, so a chunk re-encrypted after a crash produces no orphan blob. A nonce only ever encrypts one plaintext, since `mac_i` covers both the position and the plaintext.

### Chunk table

- For each chunk in order: `SHA-256(blob_i)` (32 bytes) followed by `mac_i` (32 bytes).
- An external table is stored as a blob: an envelope with key `file_table`, the first 24 bytes of `HMAC(file_nonce, "lupine/v1/table" || table)` as nonce, and `AAD = "lupine/v1/table"`. Like a chunk, a table sealed again gives the same blob.

### Reading

- A reader that needs a whole chunk fetches the blob, checks its SHA-256 and decrypts every segment.
- A reader that needs part of a chunk fetches only the byte range of the segments involved, and each segment authenticates itself.
- In both cases the reader then drops the padding, using `size`.

## Blossom

- Upload with `PUT /<sha256>` (BUD-13), falling back to `PUT /upload` (BUD-02) when `OPTIONS /<sha256>` does not advertise `PUT` (BUD-01 feature discovery), with `Content-Type: application/octet-stream`. Servers that only host media refuse this type, so a drive needs servers that store arbitrary blobs.
- A server that advertises `POST /<sha256>` (BUD-13) may get a blob by mirroring it from another drive server rather than by an upload. Either way the client checks the blob descriptor's `sha256`.
- A client uploads every blob of a version to every server before publishing it. Right before publishing, it checks with `HEAD /<sha256>` (BUD-01) that each blob the version references is still on every server, and uploads the missing ones: another device may have collected a blob that this version reuses.
- Partial reads use `GET /<sha256>` with a `Range` header (BUD-01). When a server ignores `Range` and returns the whole blob, the client keeps the part it needs.
- Authorization (BUD-11): kind 24242 signed by the drive key, with a `t` tag, the `x` tag of the blob, a `server` tag holding the server's host name, and a short expiration. Uploads, mirrors and deletions always carry a token; reads carry one once a server answers `401`. An upload token may carry several `x` tags.
- Deletion is `DELETE /<sha256>` (BUD-12). A `404` counts as deleted.
- A client deletes a blob (BUD-12) once it has seen no version on any drive relay reference it for 30 days, and checks again, after a full sync, right before deleting it. A blob that no version ever referenced, from an upload whose version was never published, is deleted by the device that uploaded it, 30 days after the upload.

## What servers learn

Relays see gift wraps addressed to the drive pubkey, like NIP-17 direct messages: signed by throwaway keys, with random timestamps, and served only to the drive by relays that follow NIP-17. They can count the wraps, see when they arrive and see deletions. They cannot read them or tell which wraps belong to the same node.

Size alone does not set a version apart from a DM, since none is smaller than a one-word message. The distribution still differs: most versions have exactly that size.

Blossom servers see the number and sizes of blobs: only the last chunk of a file is not full-size, and its size is bucketed. Range requests also show which parts of a blob are read, and the blobs a new version keeps show which chunks of a file changed. Nobody sees names, folder structure or content.

Traffic analysis can still single out a drive: a pubkey without a profile that receives thousands of wraps in bursts, and blobs of exactly 8 MiB plus 2,048 bytes.

The descriptor reveals that an npub has a Lupine drive, not which one. The drive's kinds 10050 and 10063 also go to the user's relays, so those relays can guess from timing that the two pubkeys are linked.

## What apps and devices can do

Any app the user lets decrypt with their key, through a NIP-46 bunker or a NIP-07 extension, can read the descriptor, and with it the whole drive. A signer decrypts NIP-44 payloads without knowing what they hold, so no per-kind permission protects the descriptor: only apps the user trusts with the drive should get NIP-44 decryption.

A device or a recovery key that holds the drive secret can read, write and delete everything: wraps through deletions as their recipient, blobs through BUD-12. Key rotation takes this power back, within the limits it states.
