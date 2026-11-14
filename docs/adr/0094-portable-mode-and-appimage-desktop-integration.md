# ADR 0094: Portable mode and AppImage desktop integration

* Status: accepted (2026-11-13)
* Relates to: roadmap M8 (installers), ADR 0026 (packaging layout), ADR 0062 (opening documents
  from the OS), ADR 0015 (autosave), ADR 0059 (recent files), ADR 0004 (destructive actions),
  ADR 0006 (cross-platform)

## Context

ADR 0026 shipped a Windows portable zip and a Linux tar.gz that still kept their settings in the
user's profile, and an AppImage that nothing adds to the applications menu: double-clicking a
`.anicraft` did nothing until the user wrote a `.desktop` file by hand. Artists carry their setup
between studio and home machines on a stick, and an AppImage user expects the app to offer to
integrate once, as AppImageLauncher-style tools do. App stores (Flathub, winget, the Microsoft
Store) reject metadata errors at submission time, which is late.

## Decision

1. **One switch for every folder.** `platform::placement()` decides once, at first use, whether
   this copy is portable; `config_dir()` returns the portable data folder when it is, and every
   other folder (preferences, `Recovery`, `Logs`, `plugins`, `Templates`, `Thumbnails`) already
   hangs off `config_dir()`. Recent files, brushes, filter presets and workspaces are in
   `prefs.json`, so they follow. The offline user guide goes to `data/Cache`. The environment
   overrides (`ANICRAFT_RECOVERY_DIR`, `ANICRAFT_LOG_DIR`, `ANICRAFT_TEMPLATES_DIR`) still win.
2. **A marker file, not a guess.** A copy is portable when a file named `anicraft-portable`
   (contents ignored) is found: next to the program; above `bin/` (the tar.gz layout); next to
   `AniCraft.app` on macOS (never inside the bundle, which would break its signature); next to the
   `.AppImage` (`$APPIMAGE`), not inside its read-only mount. Data goes to `data/` beside the
   marker. The Windows zip and the Linux tar.gz ship the marker; the MSI, DMG, deb, rpm, Flatpak
   and Arch package never have one (the Flatpak and Arch builds copy only `bin/` and `share/` from
   the tarball, checked in the shared workflow's scripts). Guessing from the install location
   (Program Files, `/usr`) was rejected: it misfires on custom install folders and can't be
   switched off by the user, whereas deleting the marker can.
3. **Unwritable portable folders fall back, visibly.** If `data/` can't be created and written (a
   probe file), the user folder is used and Preferences › About says why, rather than running
   with settings that silently never save.
4. **Desktop integration is engine commands.** `desktop.integrate {appimage, data_home}`,
   `desktop.remove {data_home}` and `desktop.status` (`crates/engine/src/cmds_desktop.rs`) so the
   CLI and MCP can do it too. They write exactly: `applications/ai.storyteller.anicraft.desktop`
   (the packaged entry with `Exec`/`TryExec` pointing at the AppImage and an `X-AniCraft-AppImage`
   key marking it as ours), `mime/packages/ai.storyteller.anicraft.xml` (the packaged MIME types)
   and the hicolor icons (16–256 px and the SVG), then run `update-mime-database` and
   `update-desktop-database` when installed (bounded to 5 s each, output ignored). The packaged
   desktop and MIME files and the icons are compiled in from `packaging/linux` and
   `assets/app-icon`, so there is one source.
   * The entry keeps the app id as its file name, so Wayland docks match the running window
     (`app_id`) to it. It shadows a system package's entry while present; removing it brings
     that one back.
   * `Exec` quotes the path as the Desktop Entry Specification says (always double quotes;
     `"`, `` ` ``, `$`, `\` escaped; `%` doubled; then string-value escaping), so spaces, quotes
     and dollar signs in folder names work. Paths with line breaks or that aren't UTF-8 are
     refused.
   * **Never system folders:** `data_home` must be absolute and outside `/usr`, `/etc`, `/opt`,
     `/var`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/boot`, `/snap`. The desktop app passes
     `$XDG_DATA_HOME` (a relative one is ignored, per the XDG Base Directory spec) or
     `~/.local/share`.
   * **Idempotent and safe to undo:** writing again rewrites only files that differ (temp file +
     rename); removing deletes only our file names, and nothing at all when the entry there
     wasn't written by AniCraft (no marker key), which `integrate` also refuses to replace.
   * Plug-ins may not call `desktop.*` (`plugin_may_call`).
5. **Offered once, never modal.** On the first start from an AppImage (not portable, no entry
   yet) a card at the bottom right asks "Add AniCraft to your applications menu?" with **Add to
   Menu** and **Not Now**. Either answer is kept (`desktop_offered` in the preferences); editing
   never waits on it. Preferences › About has Add / Remove / Use This Copy at any time. An entry
   that points at an AppImage which no longer exists (moved, or replaced by an update) is
   re-pointed at the running copy without asking: the user already said yes. Adding or removing
   is a routine, reversible edit of the user's own folder (ADR 0004): no confirmation.
6. **Store readiness is a check, not a submission.** `packaging/check-store-readiness.sh` runs in
   Packaging lint (`store-readiness` job, Ubuntu with `appstream`, `desktop-file-utils` and
   `python3-yaml`): both Flatpak manifests parse, carry the app id, command and required keys,
   agree on runtime, SDK and permissions, and ask for no `home`/`host` file system access; the
   filled-in AppStream metainfo passes `appstreamcli validate --no-net`; the desktop entry passes
   `desktop-file-validate`; winget and MSIX manifests, when the team adds them under
   `packaging/windows/{winget,msix}`, have their required fields. Errors fail; a missing tool is
   a warning, so the script also runs on a laptop.

## Consequences

* A zip or tarball unpacked to a stick keeps everything on the stick. Existing users of the zip
  start with fresh preferences in `data/` (their old ones stay in `%APPDATA%\AniCraft`); the
  guide says how to copy them over or delete the marker.
* AppImage users get a menu entry and file associations with one click, and lose them with one.
* The engine and CLI carry about 270 KB more (the icons and two text files).

## Not in this slice

* Making `mimeapps.list` name AniCraft the default for `.anicraft` (it is the only handler, so
  desktops pick it anyway); `.fla`/`.xfl` stay "Open with" as in ADR 0026.
* Self-update inside a portable copy (craft-update keeps its own state folder) and moving
  existing settings into a new portable copy.
* Submitting to Flathub, winget or the Microsoft Store, and committing winget/MSIX manifests.
* Running `flatpak-builder-lint` (needs the Flathub tooling on the runner).

## Alternatives considered

* **The sibling apps' `AppRun` shell script that integrates on every start.** Rejected: it writes
  without asking, can't be undone from the app, and runs before the app can show anything.
* **A different desktop file name (`appimagekit_<hash>-anicraft.desktop`).** Rejected: the window
  would no longer match its launcher on Wayland, and the dock would show a generic icon.
* **Detecting portable from the install path.** See decision 2.

## References

* freedesktop.org *Desktop Entry Specification* 1.5 (Exec quoting, string escapes, `TryExec`,
  `X-` keys), *Shared MIME-info Database* 2.4, *XDG Base Directory Specification* 0.8, *Icon Theme
  Specification* (hicolor), *AppStream* metainfo; AppImage documentation on `$APPIMAGE` and
  desktop integration; winget manifest schema 1.6 and the MSIX `AppxManifest` schema (public
  docs). No code copied.
