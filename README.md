# Lupine

End-to-end encrypted drive on Nostr and Blossom. Native apps backed by a shared Rust core; macOS comes first.

Early development: the content format, the Blossom client, the Nostr layer and the local database exist, and Finder shows a drive read-only. Writing from Finder and folder sync do not yet. See [docs/architecture.md](docs/architecture.md) and [docs/protocol.md](docs/protocol.md).

## Layout

```
crates/lupine-core   formats, crypto, transfers, Blossom and Nostr clients, local database
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

## Try the read path

Finder cannot write to a drive yet, so the `seed` example creates one from a local folder and prints its recovery key. The relays and Blossom servers are yours to pick; the servers must store arbitrary blobs, which media hosts refuse.

```sh
cargo run --release -p lupine-core --example seed -- \
  --relay wss://<relay> --server https://<blossom-server> <folder>
```

Paste the recovery key into the Lupine menu, under Drive, then open Lupine in the Finder sidebar.
