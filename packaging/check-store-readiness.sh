#!/usr/bin/env bash
# Store readiness (ADR 0094): checks the metadata app stores read, without submitting anything.
# Fails on errors; tools that aren't installed are skipped with a warning (CI installs them).
#
#   Flatpak (Flathub)  both manifests parse, name the app id, share runtime, SDK and permissions,
#                      and ask for no broad file system access (home, host)
#   AppStream          the metainfo, filled in as the packages fill it, passes
#                      `appstreamcli validate --no-net` (errors fail; warnings are shown)
#   Desktop entry      `desktop-file-validate`
#   winget / MSIX      the required fields of any manifests under packaging/windows/{winget,msix}
#                      (none are committed yet; submissions are the team's job)
#
# Usage: packaging/check-store-readiness.sh      Needs: python3 (with PyYAML for Flatpak).
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
LINUX="$ROOT/packaging/linux"
WIN="$ROOT/packaging/windows"
FAILED=0
fail() { echo "error: $*" >&2; FAILED=1; }
skip() { echo "warning: $* (skipped)" >&2; }

echo "==> Flatpak manifests"
if python3 -c 'import yaml' 2>/dev/null; then
  python3 - "$APP_ID" "$LINUX/flatpak/$APP_ID.yml" "$LINUX/flatpak/$APP_ID.bundle.yml" <<'PY' || FAILED=1
import sys, yaml
app_id, paths = sys.argv[1], sys.argv[2:]
bad = 0
def err(msg):
    global bad
    print(f"error: {msg}", file=sys.stderr)
    bad = 1
docs = {}
for p in paths:
    try:
        with open(p, encoding="utf-8") as f:
            docs[p] = yaml.safe_load(f)
    except Exception as e:
        err(f"{p}: does not parse: {e}")
        continue
    m = docs[p]
    if not isinstance(m, dict):
        err(f"{p}: not a mapping")
        continue
    for key in ("id", "runtime", "runtime-version", "sdk", "command", "modules", "finish-args"):
        if key not in m:
            err(f"{p}: missing '{key}'")
    if m.get("id") != app_id:
        err(f"{p}: id is {m.get('id')!r}, not {app_id}")
    if m.get("command") != "anicraft":
        err(f"{p}: command is {m.get('command')!r}, not anicraft")
    for arg in m.get("finish-args") or []:
        if arg in ("--filesystem=home", "--filesystem=host", "--filesystem=host-os", "--device=all"):
            err(f"{p}: {arg} is broader than Flathub accepts for this app")
    if not isinstance(m.get("modules"), list) or not m.get("modules"):
        err(f"{p}: no modules")
if len(docs) == 2:
    a, b = docs.values()
    for key in ("runtime", "runtime-version", "sdk", "finish-args"):
        if isinstance(a, dict) and isinstance(b, dict) and a.get(key) != b.get(key):
            err(f"the two manifests differ in '{key}'")
sys.exit(bad)
PY
else
  skip "python3 with PyYAML is not installed"
fi

echo "==> AppStream metainfo"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
META="$WORK/$APP_ID.metainfo.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$ANICRAFT_BUILD_DATE/g" "$LINUX/$APP_ID.metainfo.xml.in" >"$META"
if grep -q '@[A-Z_]*@' "$META"; then
  fail "the metainfo still has a placeholder after filling in VERSION and DATE"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$META" || fail "appstreamcli validate found errors in the metainfo"
else
  skip "appstreamcli is not installed"
fi

echo "==> Desktop entry"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$LINUX/$APP_ID.desktop" || fail "desktop-file-validate found errors"
else
  skip "desktop-file-validate is not installed"
fi

echo "==> winget and MSIX manifests"
found=0
if [[ -d "$WIN/winget" ]]; then
  while IFS= read -r -d '' f; do
    found=1
    for key in PackageIdentifier PackageVersion ManifestType ManifestVersion; do
      grep -Eq "^${key}:[[:space:]]*[^[:space:]]" "$f" || fail "$f: no $key"
    done
    if grep -Eq '^ManifestType:[[:space:]]*defaultLocale' "$f"; then
      for key in PackageLocale Publisher PackageName License ShortDescription; do
        grep -Eq "^${key}:[[:space:]]*[^[:space:]]" "$f" || fail "$f: no $key"
      done
    fi
    if grep -Eq '^ManifestType:[[:space:]]*installer' "$f"; then
      grep -Eq 'InstallerUrl:[[:space:]]*https://' "$f" || fail "$f: no https InstallerUrl"
      grep -Eq 'InstallerSha256:[[:space:]]*[0-9A-Fa-f]{64}' "$f" || fail "$f: no InstallerSha256"
    fi
  done < <(find "$WIN/winget" -name '*.yaml' -print0)
fi
if [[ -d "$WIN/msix" ]]; then
  while IFS= read -r -d '' f; do
    found=1
    for attr in 'Identity Name=' 'Publisher=' 'Version=' 'ProcessorArchitecture='; do
      grep -q "$attr" "$f" || fail "$f: no $attr"
    done
    for el in DisplayName PublisherDisplayName Logo; do
      grep -q "<$el>" "$f" || fail "$f: no <$el>"
    done
  done < <(find "$WIN/msix" -name 'AppxManifest.xml' -print0)
fi
[[ "$found" = 1 ]] || echo "no winget or MSIX manifests in packaging/windows yet"

if [[ "$FAILED" != 0 ]]; then
  echo "==> store readiness: errors above" >&2
  exit 1
fi
echo "==> store readiness: ok"
