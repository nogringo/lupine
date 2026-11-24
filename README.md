# Lupine

End-to-end encrypted drive on Nostr and Blossom. Native apps backed by a shared Rust core; macOS comes first, then the web.

Early development: the content format, the Blossom client, the Nostr layer and the local database exist, Finder reads and writes a drive, folders of the Mac sync with it, and the app signs in with Nostr and can change the drive key. The web app signs in, browses a drive, uploads, downloads and edits it. See [docs/architecture.md](docs/architecture.md) and [docs/protocol.md](docs/protocol.md).

## Layout

```
crates/lupine-core   formats, crypto, transfers, Blossom and Nostr clients, local database, folder sync
crates/lupine-ffi    UniFFI bindings for Swift and Kotlin
crates/lupine-wasm   WebAssembly bindings for the web app
tools/uniffi-bindgen binding generator
apple/               macOS app and File Provider extension
web/                 web app, React and TypeScript
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

The web app needs [wasm-pack](https://github.com/drager/wasm-pack), Node.js, and an archiver that indexes WebAssembly objects for SQLite's C code, which macOS's `ar` does not:

```sh
rustup target add wasm32-unknown-unknown
rustup component add llvm-tools
scripts/build-web-core.sh          # after every Rust change
cd web && npm install && npm run dev
```

## Try it

At first launch, Lupine opens a window to sign in: with a signer app (NIP-46, a QR code or a `bunker://` link) or with an nsec. It then opens your drive, or creates one. A new drive starts from your own inbox relays and Blossom servers, which you can change; the servers must store arbitrary blobs, which media hosts refuse, and Lupine checks that they do.

Lupine then shows up in the Finder sidebar. What Finder changes there reaches the relays while the Lupine app runs. To sync a folder of the Mac, choose Add Folder... in the menu. It syncs with the drive folder of the same name at the drive root, made when there is none. The menu also opens the settings, which show the recovery key, change the drive key and sign out. Changing the key takes your Nostr key once; the other devices then ask you to sign in again.

The web app offers the same sign-in, a NIP-07 browser extension included, then shows the drive as a list of folders and files: upload by picking or dropping files and folders, download by opening a file, and rename, move or delete from each item's menu. Several tabs can show the drive at once. Its Blossom servers must allow requests from web pages (CORS).

To try a drive without a Nostr account, the `seed` example creates one holding a copy of a local folder, empty or not, and prints its recovery key, which both sign-in screens open.

```sh
cargo run --release -p lupine-core --example seed -- \
  --relay wss://<relay> --server https://<blossom-server> <folder>
```
