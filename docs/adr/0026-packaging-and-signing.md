# ADR 0026: Packaging layout and signing policy

* Status: accepted (2026-11-09)
* Relates to: roadmap M8 (installers and signing), ADR 0004 (destructive actions), ADR 0006
  (cross-platform), ADR 0009 (XFL import), ADR 0015 (crash recovery), ADR 0017 (plug-ins)

## Context

AniCraft had a release workflow that calls the Craft family's shared one
(`storytold/craft-common-actions/.github/workflows/release.yml`) and a Packaging lint workflow,
but no `packaging/` folder, so neither could do anything and Packaging lint failed on every pull
request that touched a workflow. The shared release workflow is a contract: it runs
`packaging/windows/package.ps1`, `packaging/macos/{import-cert,package,verify}.sh` and
`packaging/linux/{package,flatpak-bundle,flatpak-oci}.sh` with fixed arguments and collects
`dist/release/*`; it finds the app's build variables (`ANICRAFT_VERSION`, `_BUILD_DATE`,
`_BUILD_SHA`) through `packaging/env.sh`. The sibling apps (PhotoCraft, FilmCraft, VectorCraft,
SoundCraft, …) share one layout and one installer policy. Artists coming from Animate expect a
normal installer per OS, an app that opens when they double-click a document, and no admin
prompt they didn't ask for.

## Decision

1. **The family layout, not our own.** `packaging/` follows the sibling apps file for file, so
   the shared workflows, the shared lint and anyone who knows one Craft app work unchanged:
   `env.sh` (version from `[workspace.package]`, `ANICRAFT_*` build variables, `APP_ID`),
   `check-icons.sh`, `windows/`, `macos/`, `linux/` (with `flatpak/`). The family-generic files
   (`check-install-policy.ps1`, `test-install-policy.ps1`, `verified-download.sh`,
   `postinst.sh`, the macOS signing helpers) are first-party copies from the same organisation,
   adapted only in names. FreeBSD uses the shared workflow's built-in fallback (no
   `packaging/freebsd/`), and there is no web package yet (`has-web: false`).
2. **One app id: `ai.storyteller.anicraft`.** It names the desktop entry, AppStream component,
   MIME package, hicolor icons (already in `assets/app-icon/`), the Flatpak and the macOS bundle,
   and is now also the window's Wayland app_id / X11 class (`platform::APP_ID`, was `anicraft`),
   so docks match the running window to the installed launcher and its icon.
   `src/packaging_tests.rs` fails `cargo test` if any of them drifts.
3. **What each OS gets.**
   * **Windows:** a WiX v5 MSI (x64 and arm64) with the family install policy: per-user into
     `%LOCALAPPDATA%\Programs\ArtCraft\AniCraft` by default with no admin prompt, "everyone"
     (Program Files, elevated) as an explicit choice on the first page, upgrades in place,
     cross-scope duplicates refused (`install-policy.wxi`, checked on the compiled MSI by
     `check-install-policy.ps1`, and on fixture builds in Packaging lint). Start menu shortcut,
     App Paths (Win+R `anicraft`), Finish-page "Launch AniCraft". Plus a portable zip and an
     optional symbols zip (PDBs). The UpgradeCode `95D4D208-…` and the app component GUID never
     change.
   * **macOS:** a universal `AniCraft.app` (Info.plist with the document types, hardened runtime,
     empty entitlements) on a drag-to-Applications DMG with a laid-out Finder window, and a CLI
     zip. The `.icns` is built at package time from the committed padded tile with the system's
     `sips` and `iconutil`, so no generated binary is committed for it.
   * **Linux:** AppImage (x86_64, aarch64), deb and rpm (x86_64, aarch64, riscv64), a tar.gz,
     then (in the shared workflow, from that tarball) Flatpak and Arch packages. Every format
     installs the desktop entry, AppStream metainfo, hicolor icons and MIME types. The Flatpak
     asks for display, GPU, sound (`pulseaudio`) and the Documents, Pictures and Videos folders;
     not network, not home.
4. **Documents.** `.anicraft` (`application/x-anicraft`, a JSON sub-class on Linux, UTI
   `ai.storyteller.anicraft.document`) is AniCraft's own: AniCraft is its default app on every OS.
   `.fla` and `.xfl` are offered as an alternative only ("Open with"; macOS role Viewer, rank
   Alternate), never taken over: they open as an untitled import (ADR 0009), so Save never
   writes them back. Their type names are neutral ("FLA document"), never Adobe's marks. The
   list is the Open dialog's own (`platform::filters("file.open")`), and the packaging test
   checks every entry is registered on all three systems.
5. **Signing policy.** Releases are built only by CI, from the `release` branch, with secrets in
   the repository's `release` environment: Windows Authenticode (a PFX or Azure Trusted Signing,
   `sign.ps1`), macOS Developer ID with hardened runtime, notarization and stapling
   (`package.sh`, then `verify.sh`), and the OpenPGP `LINUX_SIGNING_KEY` for the Linux packages,
   the Flatpak and `SHA256SUMS.txt`. **On `release`, a missing Windows or macOS secret fails the
   run** (the shared workflow's guard), so an unsigned build can't be published by accident.
   **Anywhere else** (a manual run, a fork, a local build) the scripts warn and produce unsigned
   or ad-hoc-signed artifacts, so the packaging can always be tried. No secret is ever read by a
   script that doesn't use it or printed.
6. **Nothing is published by CI.** The release workflow ends in a *draft* GitHub Release with
   every artifact and `SHA256SUMS.txt`; a maintainer publishes it, which creates the `v<version>`
   tag (`docs/release.md`).
7. **Release builds fail rather than degrade** where we can check: `ANICRAFT_REQUIRE_WINRES=1`
   (set by `package.ps1`) makes a missing icon/version resource in `anicraft.exe` a build error;
   `package.ps1` checks each exe's PE machine and subsystem (GUI app, console CLI); every package
   script ends with `anicraft-cli --version` (new, with `anicraft --version`). The subsystem
   check found that `anicraft.exe` was a console program (a console window opened beside the
   app): release builds are now Windows GUI programs (`windows_subsystem = "windows"`), debug
   builds keep the console. On Windows a release `anicraft --version` therefore prints only
   when its output is redirected; `anicraft-cli` is the command-line front end.

## Consequences

* A push to `release` produces installable, signed artifacts for every platform once the
  secrets are set; Packaging lint (actionlint, shellcheck, PowerShell parse, XML, desktop-file,
  AppStream and Flatpak checks, plus the Windows install-policy fixture build) guards every PR
  that touches packaging or workflows.
* The window's app id changed from `anicraft` to `ai.storyteller.anicraft`. Nothing persisted
  depends on it (eframe persistence is off; settings live in `platform::config_dir()`); window
  rules a user wrote for the old class need the new one.
* Uninstalling removes program files and registrations only. Preferences, plug-ins and
  crash-recovery copies are the user's (ADR 0004, ADR 0015) and stay.

## Not in this slice

* **macOS: opening a document from Finder.** The document types are declared (icons, "Open
  With"), but macOS hands the file to a running app as an Apple Event, not on the command line,
  and winit 0.30 doesn't deliver it; double-clicking a `.anicraft` starts AniCraft without
  opening the file. Needs an open-documents hook in the shell without `unsafe`.
  **Done in ADR 0062:** the open-documents and quit Apple Events reach the shell through
  `platform.rs`, at launch and while running, and open through the unsaved-changes guard.
  A test covers Windows' and Linux's command-line hand-over. The Mac-only checks are listed
  there.
* A version override for manual release runs (the family's `cargo xtask version set`): AniCraft
  has no `xtask` yet, so the dispatch input was removed; a manual run builds the `Cargo.toml`
  version.
* AppImage desktop integration on first run (the siblings' `AppRun` script), Windows portable
  mode (settings next to the exe), MSIX/winget/Homebrew/Flathub submission, CLI man pages and
  shell completions, a WebAssembly build.
* Video export inside the Flatpak: the sandbox can't run the host's ffmpeg (the runtime's
  ffmpeg extension is a follow-up).

## Alternatives considered

* **Our own layout or tools (cargo-dist, cargo-bundle, cargo-wix).** Rejected: the shared
  workflow and lint are the family's contract and already handle signing, notarization, Linux
  repository signing, Flatpak OCI and Arch; a second toolchain would diverge from every sibling.
* **Per-machine MSI by default** (Program Files, admin prompt). Rejected for the family policy:
  artists on managed or shared machines often can't elevate; "everyone" stays one click away.
* **Taking over `.fla`/`.xfl` as the default app.** Rejected: users with Animate installed must
  keep their default, and AniCraft imports these files rather than editing them in place.
* **Committing an `.icns`.** Rejected: it can only be made reliably on a Mac and would be one
  more generated binary; the macOS runner has `sips` and `iconutil`.
