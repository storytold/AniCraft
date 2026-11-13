#!/usr/bin/env bash
# Builds the web editor into web/ (ADR 0036, docs/web.md): index.html, anicraft.js,
# anicraft_bg.wasm and the page icon. Everything in web/ is generated (git-ignored); serve the
# folder with any static web server.
#
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI at the version Cargo.lock
# pins for the wasm-bindgen crate (both MIT OR Apache-2.0):
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the version this script asks for> --locked
#
#   scripts/build-web.sh          # optimized (the `web` profile)
#   scripts/build-web.sh --dev    # quicker to build, larger and slower
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE=web
case "${1:-}" in
  "") ;;
  --dev) PROFILE=dev ;;
  *) echo "usage: scripts/build-web.sh [--dev]" >&2; exit 2 ;;
esac

# The CLI and the crate must be the same version, or the generated glue doesn't match.
WANT="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)"
[[ -n "$WANT" ]] || { echo "error: Cargo.lock names no wasm-bindgen version" >&2; exit 1; }
command -v wasm-bindgen >/dev/null || { echo "error: wasm-bindgen not found: cargo install wasm-bindgen-cli --version $WANT --locked" >&2; exit 1; }
HAVE="$(wasm-bindgen --version | awk '{ print $2 }')"
[[ "$HAVE" = "$WANT" ]] || { echo "error: wasm-bindgen $HAVE found, Cargo.lock needs $WANT: cargo install wasm-bindgen-cli --version $WANT --locked" >&2; exit 1; }
if command -v rustup >/dev/null && ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
  echo "error: the wasm32-unknown-unknown target is missing: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi

cargo build -p anicraft --bin anicraft --target wasm32-unknown-unknown --profile "$PROFILE"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIR="$PROFILE"
[[ "$PROFILE" = dev ]] && DIR=debug
WASM="$TARGET_DIR/wasm32-unknown-unknown/$DIR/anicraft.wasm"
OUT="$ROOT/web"
rm -rf "$OUT"
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name anicraft "$WASM"
cp packaging/web/index.html "$OUT/index.html"
cp assets/app-icon/hicolor/128x128/apps/ai.storyteller.anicraft.png "$OUT/anicraft.png"
# The user guide beside the editor: Help › User Guide opens guide/index.html (ADR 0069).
cargo run -q -p anicraft-cli -- guide --out "$OUT/guide"

BYTES="$(wc -c <"$OUT/anicraft_bg.wasm" | tr -d ' ')"
GZIP="$(gzip -9 -c "$OUT/anicraft_bg.wasm" | wc -c | tr -d ' ')"
echo "web/ is ready: anicraft_bg.wasm is $((BYTES / 1024)) KB ($((GZIP / 1024)) KB gzipped)."
echo "Serve it locally, for example: python3 -m http.server --directory web 8080  (then open http://localhost:8080)"
