#!/usr/bin/env bash
# Builds the Rust core for Apple platforms and packages it for SwiftPM:
# apple/LupineCore/LupineFFI.xcframework plus the generated Swift bindings.
# Run it before opening the Xcode project, and again after changing Rust code.
#
#   scripts/build-apple-core.sh            # release
#   PROFILE=dev scripts/build-apple-core.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${PROFILE:-release}"
PROFILE_DIR="$([ "$PROFILE" = dev ] && echo debug || echo "$PROFILE")"
PACKAGE="$ROOT/apple/LupineCore"
WORK="$ROOT/target/apple"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

export MACOSX_DEPLOYMENT_TARGET=14.0

cd "$ROOT"
for target in "${TARGETS[@]}"; do
  cargo build -p lupine-ffi --lib --profile "$PROFILE" --target "$target"
done

rm -rf "$WORK"
mkdir -p "$WORK/bindings" "$WORK/headers" "$WORK/macos"

host_dylib="target/aarch64-apple-darwin/$PROFILE_DIR/liblupine_ffi.dylib"
cargo run -q -p uniffi-bindgen -- generate \
  --library "$host_dylib" --language swift --out-dir "$WORK/bindings"

cp "$WORK/bindings/LupineFFI.h" "$WORK/headers/"
cp "$WORK/bindings/LupineFFI.modulemap" "$WORK/headers/module.modulemap"

lipo -create \
  "target/aarch64-apple-darwin/$PROFILE_DIR/liblupine_ffi.a" \
  "target/x86_64-apple-darwin/$PROFILE_DIR/liblupine_ffi.a" \
  -output "$WORK/macos/liblupine_ffi.a"

rm -rf "$PACKAGE/LupineFFI.xcframework"
xcodebuild -create-xcframework \
  -library "$WORK/macos/liblupine_ffi.a" -headers "$WORK/headers" \
  -output "$PACKAGE/LupineFFI.xcframework" >/dev/null

cp "$WORK/bindings/LupineCore.swift" "$PACKAGE/Sources/LupineCore/LupineCore.swift"
echo "LupineCore ready: $PACKAGE"
