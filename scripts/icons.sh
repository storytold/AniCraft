#!/usr/bin/env bash
# Regenerate every app icon file from assets/app-icon/anicraft.svg (the canonical artwork).
#
# Needs: resvg (cargo install resvg) and Python with Pillow (for the .ico). The outputs are
# committed, so building never needs these tools.
#
#   scripts/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/anicraft.svg"
ID="ai.storyteller.anicraft"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (cargo install resvg)" >&2; exit 1; }

# The SVG is a full-bleed 512 tile, used as-is on Windows and Linux. macOS icons follow Apple's grid:
# the tile is 824/1024 of the canvas with a transparent margin, so widen the viewBox around it.
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 512 512"/viewBox="-62.136 -62.136 636.272 636.272"/' "$SVG" >"$MAC"
grep -q 'viewBox="-62.136' "$MAC" || { echo "error: $SVG has no 512 viewBox" >&2; exit 1; }

render() { resvg -w "$2" -h "$2" "$1" "$3" </dev/null; }

render "$SVG" 1024 "$DIR/anicraft-1024.png"
# The tile on Apple's grid: the runtime Dock icon (src/main.rs) and store/landing pages.
render "$MAC" 1024 "$DIR/anicraft-padded-1024.png"
render "$MAC" 512 "$DIR/anicraft-macos-512.png"

# Linux hicolor theme (the 256 one is also the runtime icon on Windows and Linux, the 128 one
# the mark in Help › About).
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$SVG" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$DIR/anicraft-small.svg" "$DIR/hicolor/scalable/apps/$ID.svg"

# Windows .ico (embedded in anicraft.exe by build.rs): each size rendered, not scaled down.
ICO_SIZES=(16 20 24 32 40 48 64 128 256)
for s in "${ICO_SIZES[@]}"; do render "$SVG" "$s" "$TMP/ico-$s.png"; done
python - "$DIR/anicraft.ico" "$TMP" "${ICO_SIZES[@]}" <<'EOF'
import sys
from PIL import Image
out, tmp, sizes = sys.argv[1], sys.argv[2], [int(s) for s in sys.argv[3:]]
imgs = [Image.open(f"{tmp}/ico-{s}.png").convert("RGBA") for s in sizes]
imgs[-1].save(out, format="ICO", sizes=[(s, s) for s in sizes], append_images=imgs[:-1])
EOF
echo "icons written to $DIR"
