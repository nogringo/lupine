#!/usr/bin/env bash
# Builds the Rust core for the web app: web/core holds the WebAssembly module
# and its TypeScript bindings. Run it before starting the web app, and again
# after changing Rust code.
#
#   scripts/build-web-core.sh            # release
#   PROFILE=dev scripts/build-web-core.sh
#
# SQLite's C code needs an archiver that indexes WebAssembly objects, which
# macOS's ar does not: `rustup component add llvm-tools` provides one.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${PROFILE:-release}"

if [ -z "${AR_wasm32_unknown_unknown:-}" ]; then
  host="$(rustc -vV | sed -n 's/^host: //p')"
  bundled="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-ar"
  if [ -x "$bundled" ]; then
    export AR_wasm32_unknown_unknown="$bundled"
  elif command -v llvm-ar >/dev/null; then
    export AR_wasm32_unknown_unknown="$(command -v llvm-ar)"
  else
    echo "llvm-ar not found: run rustup component add llvm-tools" >&2
    exit 1
  fi
fi

cd "$ROOT"
wasm-pack build crates/lupine-wasm --target web --no-pack \
  --out-dir "$ROOT/web/core" --out-name lupine "--$PROFILE"
echo "Web core ready: $ROOT/web/core"
