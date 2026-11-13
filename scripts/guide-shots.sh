#!/usr/bin/env bash
# Takes the user guide's screenshots (docs/guide/images, ADR 0069): the desktop app at its fixed
# window size (1600 × 940) with the sample animation, one `--open` state per picture, scaled down
# to 1100 px across. Run it after a change that alters what a picture shows, look at every PNG,
# and commit them. The pictures are original work (ATTRIBUTION.md).
#
#   scripts/guide-shots.sh            # all of them
#   scripts/guide-shots.sh library    # only the ones whose name contains "library"
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="docs/guide/images"
WIDTH=1100
ONLY="${1:-}"

cargo build -p anicraft --bin anicraft
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
APP="$TARGET_DIR/debug/anicraft"
[[ -x "$APP" || -x "$APP.exe" ]] || { echo "error: $APP was not built" >&2; exit 1; }

# picture name, then the --open state ("-" for the window as it starts)
SHOTS=(
  "workspace -"
  "start-screen start"
  "library library"
  "onion-skin timelinepolish"
  "motion-editor motion"
  "test-movie testmovieinside"
  "publish-settings publish"
  "preferences prefs"
)
mkdir -p "$OUT"
for row in "${SHOTS[@]}"; do
  read -r name state <<<"$row"
  [[ -z "$ONLY" || "$name" == *"$ONLY"* ]] || continue
  args=(--screenshot "$OUT/$name.png" --shot-width "$WIDTH" --lang en)
  [[ "$state" == "-" ]] || args+=(--open "$state")
  "$APP" "${args[@]}"
done
echo "Pictures in $OUT: look at each one before committing."
