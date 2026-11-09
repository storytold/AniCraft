#!/usr/bin/env bash
# Check that every icon file the packaging uses exists and has the size it claims, without
# building anything. packaging-lint runs it (storytold/craft-common-actions).
#
#   bash packaging/check-icons.sh
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
ICONS="$ROOT/assets/app-icon"
failed=0
need() {
  if [[ ! -s "$1" ]]; then
    echo "error: missing icon $1" >&2
    failed=1
  fi
  return 0
}

# PNG width and height are big-endian at bytes 16..23 of the IHDR chunk.
png_size() {
  od -An -tu1 -j16 -N8 "$1" | awk '{ printf "%dx%d", $1*16777216 + $2*65536 + $3*256 + $4, $5*16777216 + $6*65536 + $7*256 + $8 }'
  return 0
}

# Windows: the .ico build.rs embeds and the MSI's shortcut and Add/Remove Programs icon.
need "$ICONS/anicraft.ico"
# macOS: package.sh makes AniCraft.icns from the tile on Apple's grid.
need "$ICONS/anicraft-padded-1024.png"
# Linux: hicolor theme icons (desktop entry, AppStream, MIME type, AppImage, Flatpak).
for s in 16 24 32 48 64 128 256 512; do
  png="$ICONS/hicolor/${s}x${s}/apps/$APP_ID.png"
  need "$png"
  if [[ -s "$png" ]] && [[ "$(png_size "$png")" != "${s}x${s}" ]]; then
    echo "error: $png is $(png_size "$png"), expected ${s}x${s}" >&2
    failed=1
  fi
done
need "$ICONS/hicolor/scalable/apps/$APP_ID.svg"
if [[ -s "$ICONS/anicraft-padded-1024.png" ]] && [[ "$(png_size "$ICONS/anicraft-padded-1024.png")" != "1024x1024" ]]; then
  echo "error: anicraft-padded-1024.png is not 1024x1024" >&2
  failed=1
fi

if [[ "$failed" != 0 ]]; then exit 1; fi
echo "app icons ok"
