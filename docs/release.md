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
and an annotated tag (`v0.1.0`) on `main`.

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
  PR, and the merge commit on `main` is tagged.
