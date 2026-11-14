#!/bin/sh
# Builds the sample plug-ins to WebAssembly (docs/plugins.md) and copies them to ./dist.
# Needs the wasm32-unknown-unknown target: `rustup target add wasm32-unknown-unknown`.
# Install one with Commands › Install Plug-in…, or try them headless:
#   anicraft-cli exec --sample plugin.reload '{"folder": "examples/plugins/dist"}'
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
out="${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release"
mkdir -p dist
for name in star_field jitter text_export stepped_ease; do
    cp "$out/anicraft_plugin_$name.wasm" "dist/$name.wasm"
    echo "dist/$name.wasm: $(wc -c < "dist/$name.wasm") bytes"
done
