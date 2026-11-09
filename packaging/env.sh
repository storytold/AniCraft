# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# The family's shared release workflow (storytold/craft-common-actions) sources it too: it reads
# VERSION and finds the ANICRAFT_ prefix in the VERSION line below.
#
# Exports:
#   ROOT                   workspace root
#   VERSION                [workspace.package] version from Cargo.toml (override: ANICRAFT_VERSION)
#   DIST                   output directory for release artifacts (default: $ROOT/dist/release)
#   ANICRAFT_BUILD_SHA     git commit of the build (Info.plist, logs)
#   ANICRAFT_BUILD_DATE    UTC build date, YYYY-MM-DD (AppStream release date)
#   CARGO_TARGET_DIR       cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" \r]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
  return 0
}

VERSION="${ANICRAFT_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [[ -z "${ANICRAFT_BUILD_SHA:-}" ]]; then
  ANICRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export ANICRAFT_BUILD_SHA
export ANICRAFT_BUILD_DATE="${ANICRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# The app id, used for the desktop entry, AppStream, MIME package, icons, the macOS bundle id and
# the window's Wayland app_id / X11 class (src/platform.rs). One value everywhere.
export APP_ID=ai.storyteller.anicraft

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
  return 0
}

# Copy the licence, notice and readme files into a package directory, plus the licences of the
# craft-fonts fonts the build embedded.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  copy_font_licences "$dest"
  return 0
}

# Builds made with craft-fonts (CRAFT_FONTS_DIR, every official release) embed its OFL-1.1 fonts:
# ship each font's licence as OFL-<family-dir>.txt. Does nothing without CRAFT_FONTS_DIR.
copy_font_licences() {
  local dest="$1" lic family
  if [[ -z "${CRAFT_FONTS_DIR:-}" ]]; then return 0; fi
  for lic in "$CRAFT_FONTS_DIR"/fonts/*/OFL.txt; do
    [[ -f "$lic" ]] || continue
    family="$(basename "$(dirname "$lic")")"
    cp "$lic" "$dest/OFL-$family.txt"
  done
  return 0
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file" | cut -d' ' -f1; else shasum -a 256 "$file" | cut -d' ' -f1; fi
  return 0
}
