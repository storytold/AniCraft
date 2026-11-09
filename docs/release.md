# Releases and contributing flow

## Versions

AniCraft follows semantic versioning. Before 1.0, minor versions may change file formats and
commands, with migration on load (ADR 0003).

| Version | Stage | Meaning |
|---|---|---|
| `0.1.0` | early alpha | M0 + M1 done: authoring core, Animate file import, GIF/PNG export, editable shortcuts. |
| `0.2.0`, … | alpha | Further milestones from `docs/roadmap.md`. |

The version lives in one place, `[workspace.package]` in `Cargo.toml`, and the stage in
`RELEASE_STAGE` (`src/app.rs`). The About window, the CLI's `serve` handshake and the
`generator` field of saved files all read it. Every version gets a section in `CHANGELOG.md`
and a `v<version>` tag, created when its draft release is published (below).

## How changes land

* `main` is the development branch. It begins with one commit, "Initial commit v0.1.0 early
  alpha"; `AUTHORS.md` credits everyone whose work it contains.
* Work happens on topic branches (`feat/svg-export`, `fix/…`, `docs/…`), with one roadmap step
  per pull request. A PR is ready when build, tests, clippy (`-D warnings`) and
  `cargo deny check licenses bans sources` pass, the screenshot check looks right, and
  `docs/roadmap.md` › Honest status and `CHANGELOG.md` (Unreleased) are updated.
* PRs are merged into `main` on GitHub, and nobody commits or pushes to `main` directly (the
  hooks in `scripts/git-hooks` refuse it locally; branch protection should require PRs on the
  remote).

## Installers and packaging

`packaging/` holds every installer (ADR 0026), in the Craft family's layout, so the family's
shared release workflow builds them. `.github/workflows/release.yml` runs it on every push to
the `release` branch (and by hand from the Actions tab):

| OS | Artifacts (`dist/release/`) | Script |
|---|---|---|
| Windows x64, arm64 | `anicraft-<v>-windows-<arch>.msi`, `-portable.zip`, `-symbols.zip` | `packaging/windows/package.ps1 -Arch x64` |
| macOS universal | `anicraft-<v>-macos-universal.dmg`, `anicraft-cli-<v>-macos-universal.zip` | `packaging/macos/package.sh --arch universal`, then `verify.sh` |
| Linux x86_64, aarch64 | `.AppImage` (+ `.zsync`), `.deb`, `.rpm`, `.tar.gz`; Flatpak and Arch packages made from the tarball | `packaging/linux/package.sh`, `flatpak-bundle.sh` |
| Linux riscv64 | `.deb`, `.rpm`, `.tar.gz` (cross-compiled) | `package.sh --formats "deb rpm tar"` |
| FreeBSD x86_64, aarch64 | `.tar.gz` | the shared workflow's built-in packager |

Every installer registers `.anicraft` (AniCraft is its default app) and offers AniCraft for
`.fla` and `.xfl` under "Open with" without taking them over; installs the app icon, the
`anicraft-cli` next to the app, and our licences and notices. The Windows MSI installs for the
current account by default (`%LOCALAPPDATA%\Programs\ArtCraft\AniCraft`, no admin prompt) and
for everyone on request; see `packaging/windows/INSTALLATION.md`. Each script runs on its own OS
and writes to `dist/release/`; without signing secrets it warns and builds unsigned (Windows) or
ad-hoc-signed (macOS) artifacts, which is how to try the packaging locally.

**Signing.** The secrets live in the repository's `release` environment: `WINDOWS_CERTIFICATE`
and `WINDOWS_CERTIFICATE_PASSWORD`, or the six `AZURE_*` values for Azure Trusted Signing;
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`,
`APPLE_PASSWORD` (app-specific) and `APPLE_TEAM_ID` for Developer ID signing and notarization;
`LINUX_SIGNING_KEY` (OpenPGP) for the Linux packages, the Flatpak and `SHA256SUMS.txt`. A run on
`release` with a Windows or macOS secret missing fails before building. A run anywhere else
builds unsigned artifacts.

**What CI checks.** Packaging lint, on every PR touching `packaging/`, workflows or the app icon:
actionlint, shellcheck, PowerShell parsing, XML, `desktop-file-validate`, `appstreamcli
validate`, the Flatpak manifests and icon files, and a WiX build of the installer for x64, x86
and arm64 whose compiled scope, folder and registry rules are checked. Windows ARM64 packages the
real MSI, installs it on an ARM64 runner, runs `anicraft-cli --version` and uninstalls it.
`cargo test` (`src/packaging_tests.rs`) keeps the app id, document types, registry inventory and
icons consistent across the three systems. Only a real release run builds the macOS bundle and
DMG, the AppImage, deb, rpm and Flatpak, signs and notarizes.

**Publishing.** The workflow ends in a *draft* GitHub Release with every artifact and
`SHA256SUMS.txt`. Check the artifacts, then publish the draft: that creates the `v<version>` tag
on the release commit. CI never publishes.

## Before a release

* [ ] Counsel confirm the patent posture and the clean-room approach to opening Animate files
  (`docs/legal/compliance.md`).
* [ ] Counsel or a trademark search confirm the name "AniCraft".
* [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings` and
  `cargo deny check licenses bans sources` pass, plus screenshot checks on Windows, macOS and
  Linux.
* [ ] `ATTRIBUTION.md`, `NOTICE`, `licenses/` and `docs/legal/third-party-inventory.txt` are
  current.
* [ ] `CHANGELOG.md` has the version's section; the version is bumped in `Cargo.toml` through a
  PR, merged to `main`, and `main` is pushed to `release`.
* [ ] The release run is green and the draft has every artifact; on a clean machine per OS the
  installer installs, `.anicraft` files open AniCraft (macOS: see ADR 0026, not yet), and the
  app uninstalls cleanly. Then publish the draft (this tags `v<version>`).
