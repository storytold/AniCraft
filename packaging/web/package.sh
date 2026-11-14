#!/usr/bin/env bash
# The web editor as a release download (#210, ADR 0036, docs/web.md):
#
#   $DIST/anicraft-web-<version>.zip
#
# a static site (index.html, the loader, the module, the icon and the user guide) to unzip onto
# any web server. This is the app's side of the family release workflow's web job
# (storytold/craft-common-actions, `has-web`), which runs this script and uploads
# dist/release/*; the Web CI job runs it too, so it never breaks unnoticed.
#
# Builds with scripts/build-web.sh; installs the wasm-bindgen CLI at the version Cargo.lock pins
# when it is missing (the release runner has only Rust and the wasm32 target).
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
cd "$ROOT"
command -v zip >/dev/null || { echo "error: zip not found (apt install zip)" >&2; exit 1; }

WANT="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)"
if ! command -v wasm-bindgen >/dev/null || [[ "$(wasm-bindgen --version | awk '{ print $2 }')" != "$WANT" ]]; then
  cargo install wasm-bindgen-cli --version "$WANT" --locked
fi

scripts/build-web.sh
for f in index.html anicraft.js anicraft_bg.wasm anicraft.png guide/index.html; do
  [[ -s "web/$f" ]] || { echo "error: web/$f is missing or empty" >&2; exit 1; }
done

ZIP="$DIST/anicraft-web-$VERSION.zip"
rm -f "$ZIP"
(cd web && zip -q -r -X "$ZIP" .)
echo "$ZIP: $(( $(wc -c <"$ZIP" | tr -d ' ') / 1024 )) KB"
