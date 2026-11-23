# Lupine

End-to-end encrypted drive on Nostr and Blossom. Native apps backed by a shared Rust core; macOS comes first.

Early development: the content format, the Blossom client, the Nostr layer and the local database exist, Finder reads and writes a drive, and folders of the Mac sync with it. Sign-in does not exist yet. See [docs/architecture.md](docs/architecture.md) and [docs/protocol.md](docs/protocol.md).

## Layout

```
crates/lupine-core   formats, crypto, transfers, Blossom and Nostr clients, local database, folder sync
crates/lupine-ffi    UniFFI bindings for Swift and Kotlin
tools/uniffi-bindgen binding generator
apple/               macOS app and File Provider extension
scripts/             build scripts
```

## Build

Requirements: Rust (stable), Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen).

```sh
cargo test

rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-apple-core.sh        # after every Rust change
cd apple && xcodegen generate && open Lupine.xcodeproj
```

The File Provider extension starts disabled. Turn it on in System Settings, General, Login Items & Extensions, then Lupine shows up in the Finder sidebar.

## Try it

There is no sign-in yet, so the `seed` example creates a drive holding a copy of a local folder, empty or not, and prints its recovery key. The relays and Blossom servers are yours to pick; the servers must store arbitrary blobs, which media hosts refuse.

```sh
cargo run --release -p lupine-core --example seed -- \
  --relay wss://<relay> --server https://<blossom-server> <folder>
```

Paste the recovery key into the Lupine menu, under Drive, then open Lupine in the Finder sidebar. What Finder changes there reaches the relays while the Lupine app runs.

To sync a folder of the Mac, choose Add Folder... in the menu. It syncs with the drive folder of the same name at the drive root, made when there is none.
