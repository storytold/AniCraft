# ADR 0067: CI on macOS and Windows

* Status: accepted (2026-11-12)
* Relates to: ADR 0006 (cross-platform), ADR 0026 (packaging; the FreeBSD and Windows ARM64
  jobs), ADR 0031/0032 (GPU tests against the CPU oracle), ADR 0036 (web build), ADR 0062
  (Apple Events), ADR 0064 (system contrast); `docs/development.md` › CI

## Context

The pull-request gate (`ci.yml`, the suite's shared `gate.yml` in
`storytold/craft-common-actions`) ran on Ubuntu only. The web build has its own job, and
FreeBSD and Windows ARM64 run nightly or on packaging changes. So `cfg(target_os = "macos")`
code (the platform layer, the Apple Events listener of ADR 0062, Increase Contrast of
ADR 0064, the Mac app icon) was never compiled by CI, and `cfg(windows)` code (registry
contrast, the resource build in `build.rs`, DX12 shader paths) only on developers' machines.
A Windows developer can't cross-check macOS either: `ring` (through the updater) needs a C
toolchain for the target.

Constraints: runners are congested and the organisation has few macOS runners (pdfcraft keeps
macOS to a nightly run for that reason); the repository is public, so minutes are free but
queue time isn't. Every job must start with Harden-Runner (#162) and pin actions by SHA.

## Decisions

1. **The shared gate, with other runners, in a workflow of our own** (`platforms.yml`, "CI
   (macOS, Windows)"). `gate.yml` already takes an `os` input, so we don't copy its setup: we
   call it with `["macos-latest", "windows-latest"]` (arm64 macOS, x64 Windows). That gives
   both legs the suite's Harden-Runner configuration (audit, the shared allow list), the
   SHA-pinned checkout, craft-fonts at the same pin as `ci.yml`, the toolchain from
   `rust-toolchain.toml`, and `rust-cache` keyed per OS and architecture (written by pushes to
   `main`, read by pull requests). The reusable workflow tracks `@main` like every other call
   into craft-common-actions (suite-wide fixes land without a PR here).
2. **What each leg runs:** `cargo build --workspace --locked`, `cargo test --workspace --locked
   --no-fail-fast`, `cargo clippy --workspace --all-targets --locked -- -D warnings`. All three
   run even when one fails, so one run reports everything. No wasm target and no nextest
   (`targets: ""`, `tools: ""`): `web.yml` owns the wasm build. The dev profile's
   `line-tables-only` stays (it also keeps Windows PDBs small), so `dev-debug: ""`.
3. **GPU tests skip cleanly** without a usable adapter (`crates/gpu/src/tests.rs`), so no
   leg needs a graphics card; where the runner offers an adapter (Metal on macOS, Direct3D 12
   on Windows) they run against the CPU oracle.
4. **Separate workflow, own concurrency group.** `ci-platforms-<ref>` cancels a superseded
   pull-request run (macOS slots are scarce) but never the Linux gate, and every push to
   `main` runs to the end.
5. **Path filters.** A change that only touches `docs/**`, Markdown or `.github/**` (other
   than `platforms.yml` itself) skips both legs, on pull requests and on `main`. Mixed
   changes run. The legs are not required checks, so a skipped run never blocks a merge.
6. **Every code PR runs both legs.** Narrowing macOS to platform-sensitive paths (as
   lightcraft does for Windows) would miss the common failure, a shared crate that stops
   compiling on one OS; we start broad and narrow only if the macOS queue becomes the
   bottleneck (see Costs).

## Costs (measured on pull request #190, 2026-11-12)

First run of each leg, with no dependency cache (pull requests only read the cache `main`
writes, and this workflow had never run on `main`). Wall clock of the gate step:

| Leg | `cargo build` | test build | tests run | clippy | Job total |
|---|---|---|---|---|---|
| macOS arm64 (`macos-latest`) | 7 m 27 s | 5 m 59 s | 1 m 20 s | 1 m 13 s | **16 m 33 s** |
| Windows x64 (`windows-latest`) | 11 m 59 s | 11 m 03 s | 4 m 00 s | 1 m 48 s | **28 m 02 s** |
| for comparison: Linux gate (`ci.yml`, warm cache) | | | | | 6 m 24 s |

So a code PR costs about 45 more runner-minutes cold, run in parallel with the Linux gate
(the PR's wall time becomes the Windows leg). With `main`'s cache, most of the two build
columns (dependencies) is restored rather than compiled; record the warm numbers in
`docs/development.md` once `main` has run. Minutes are free for this public repository; the
cost that matters is queue time on the organisation's few macOS runners, which the
cancel-superseded group and the docs-only filter keep down. If the macOS queue still
becomes the bottleneck, the next step is narrowing the macOS leg to platform-sensitive paths
plus a `full-ci` label (craft-common-actions' `changed-paths.yml`), not dropping it.

The macOS leg found one real failure on its first run: the macOS app menu is titled
"AniCraft", and the pseudo-locale screen-reader tests (ADR 0040) flagged the product name as
untranslated in 50 states. The tests now treat the product name as the source scan already
does. Windows found nothing; clippy was clean on both. The 19 GPU parity tests passed on both
legs (5.7 s on macOS, 29.9 s on Windows). GNU `timeout` is missing on macOS, so the gate
warns there that `gate-timeout-minutes` can't apply; the job's own 90-minute timeout still
does.

## CodeQL and the workflow files

CodeQL's default setup (actions, Rust, Python, JS/TS) had five `startup_failure` runs on
2026-11-11 between 20:25 and 20:50 UTC, on `main` and on PRs #149 and #165. They aren't caused
by our workflows: no file under `.github/` changed between #162's merge (17:08) and the first
successful run after them (20:55, same workflow files), every CodeQL run since has
succeeded, the `actions` analysis extracts all our workflow files without errors, and there are
no open code-scanning alerts. Default setup runs GitHub's own dynamic workflow
(`dynamic/github-code-scanning/codeql`), so a startup failure there comes from GitHub or the
organisation's settings, not from files in this repository. If it comes back, an organisation
admin should check, in this order: Settings › Actions › General (the allowed-actions policy
and "require actions to be pinned to a full-length commit SHA": the dynamic CodeQL workflow
uses GitHub's own actions by tag), the runner group default setup is assigned to
(`runner_type: standard`), and the repository's Code security › CodeQL tool status page.
`platforms.yml` uses no YAML anchors or other syntax the actions extractor might not read.

## Consequences

* macOS-only and Windows-only code is compiled, linted and tested on every code PR.
* Two more checks per PR; their wall time runs in parallel with the Linux gate.
* Not yet: Linux arm64 and Windows ARM64 test legs on PRs (ARM64 packaging stays in
  `windows-arm64.yml`), a real GPU runner, `cargo deny` on these legs (it's
  platform-independent and the Linux side covers it), the screenshot check on macOS.
