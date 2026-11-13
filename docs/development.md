# Development

## Build, run, test

```sh
cargo build                                   # app + CLI (debug)
cargo run                                     # desktop app with the sample
cargo run -- --screenshot shots/now.png       # draw ~120 frames, save the window, exit
cargo run -- --open <panel> --screenshot x.png  # palette|shortcuts|files|confirm|selection|transform|points|pen|frames|span|bucket|sound|tlzoom|tlwide|prefs|licenses|zoom|editmultiple|edittransform|onionsymbol
cargo run -- --theme high-contrast --scale 200 --screenshot x.png  # a palette and interface scale for one run (not saved)
cargo test                                    # every crate
cargo clippy --all-targets -- -D warnings
cargo deny check licenses bans sources
cargo run --release -p anicraft-cli -- bench  # performance probe
```

Fonts: builds embed Inter and JetBrains Mono when `CRAFT_FONTS_DIR` points at a craft-fonts
checkout, or when one is found next to the repository (ADR 0005). Otherwise they use the
built-in fallback fonts.

Sound output (ADR 0020) uses `cpal`, which on Linux and the BSDs builds against ALSA: install
`libasound2-dev` and `pkg-config` (Debian/Ubuntu), `alsa-lib-devel` (Fedora) or `alsa-lib`
(Arch, FreeBSD). `cargo build --no-default-features` leaves sound output out (playback is
silent; import, waveforms and audio export still work). Tests never open an audio device.

### The user guide

The user guide is Markdown in `docs/guide/` (ADR 0069); `crates/guide` bundles it as HTML for
Help › User Guide, and `cargo run -p anicraft-cli -- guide --out <folder>` writes the pages.
`cargo test` checks every link, anchor and picture and that every chapter is reachable from
`index.md`; a new chapter needs its line in `PAGES` (`crates/guide/src/lib.rs`) and a link
from the contents. The command and shortcut references in `docs/guide/reference/` are
generated: when a command or a default key changes, a test fails until you run

```sh
ANICRAFT_BLESS_GUIDE=1 cargo test -p anicraft guide_reference
```

and commit the result. The pictures come from `scripts/guide-shots.sh` (or `.ps1`), which runs
`--screenshot --open <state> --shot-width 1100`; look at each one before committing it.

### GPU stage

The stage draws on the graphics card by default (ADR 0032; the shell's `gpu-stage` feature is
on by default, and the CPU draws when there is no usable card). For a CPU-only shell:
`cargo run --no-default-features --features sound-output`; to compare at run time,
`ANICRAFT_STAGE_RENDERER=cpu|gpu`. The `anicraft-gpu` crate builds and tests in every build:
its parity tests run on the graphics card, else on a software adapter (lavapipe; WARP fails the self-test, ADR 0032), and
skip without either; `ANICRAFT_TEST_GPU=software` forces the software adapter and
`WGPU_BACKEND=dx12|vulkan|metal` picks the backend. `cargo run --release -p anicraft-gpu
--example gpu_bench` prints timings and parity.

## CI

Every workflow calls the suite's shared workflows in `storytold/craft-common-actions`, which
start each job with Harden-Runner and pin actions by SHA.

| Workflow | Runs on | When | What |
|---|---|---|---|
| `ci.yml` (CI) | Ubuntu 24.04 (lavapipe for GPU tests) | every PR, every push to `main` | `cargo test --workspace`, clippy |
| `platforms.yml` (CI (macOS, Windows), ADR 0067) | `macos-latest` (arm64), `windows-latest` (x64) | PRs and pushes to `main` that change more than docs, Markdown or other workflows | `cargo build`, `cargo test --no-fail-fast`, clippy, for the workspace |
| `web.yml` | Ubuntu | every PR, every push to `main` | wasm32 build, the engine's tests in Node, the web bundle |
| `freebsd.yml`, `windows-arm64.yml` | FreeBSD VM, Windows ARM64 | nightly / weekly, packaging changes | build and tests; ARM64 MSI install |
| `sonarcloud.yml`, CodeQL default setup | Ubuntu | every PR | coverage and static analysis |

Measured job times (wall clock, from the job's start to its end):

| Job | 2026-11-12, PR #190 |
|---|---|
| Linux gate (`ci.yml`, warm cache) | 6 m 24 s |
| macOS arm64 leg (first run, no cache) | 16 m 33 s: build 7.5 min, test build 6.0, tests 1.3, clippy 1.2 |
| Windows x64 leg (first run, no cache) | 28 m 02 s: build 12.0 min, test build 11.1, tests 4.0, clippy 1.8 |

The macOS and Windows legs run in parallel with the Linux gate, so a code PR takes about as
long as the Windows leg and about 45 more runner-minutes cold (ADR 0067 › Costs).

A superseded PR run is cancelled (each workflow has its own `concurrency` group, so one never
cancels another); pushes to `main` always run to the end and are the only runs that write the
dependency caches, so a PR's first run reuses `main`'s. To start the platform legs by hand:
`gh workflow run platforms.yml --ref <branch>`.

## Build footprint (biased for disk space, then time)

Measured on Windows, 32 threads, offline, from an empty target directory:

| | Before (2026-10-28) | Now |
|---|---|---|
| Debug build of the workspace | 6.5–6.9 GB in `target/` (3.0 GB of it the incremental cache) | **1.04 GB** (1.08 GB with all test binaries; 1.33 GB with build + tests + clippy, measured after a clean) |
| Fresh build time | ~60 s | **44 s** (tests then add 3 s) |
| Rebuild after editing a UI file | ~2 s (incremental) | **3.8 s** |
| Rebuild after editing an engine file | ~3 s | **6.0 s** |
| Debug `anicraft.exe` | 21 MB + 200 MB PDB | 21 MB + small line-table PDB |

What does it (in `Cargo.toml`):

| Setting | Effect |
|---|---|
| `profile.dev.debug = "line-tables-only"` | Panics and backtraces keep file:line; full variable info (most of the PDB/DWARF size) is gone. Also avoids MSVC's PDB limit (LNK1140). Use `debug = true` locally when you need a debugger session. |
| `profile.dev.incremental = false` | The incremental cache was ~3 GB for our crates. They rebuild from scratch in seconds. |
| `profile.dev.package."*"`: `opt-level = 2, debug = false` | Dependencies are optimized once (a fast app even in debug) and carry no debug info. |
| `profile.dev.build-override`: `opt-level = 0, debug = false` | Build scripts and proc-macros compile fast and small. |
| `profile.release.strip = "debuginfo"` | Smaller release binaries. |
| eframe without default features | Drops `links` (url/idna/ICU Unicode tables) and the web-only screen-reader shim. |

House rules:

* **One target directory.** Never point agents or scripts at a second `CARGO_TARGET_DIR`: each
  copy is another full build (1+ GB now, 3–7 GB before).
* After changing profiles or dependency features, run `cargo clean` once, so the old artifacts
  don't sit next to the new ones. Close the app first: Windows can't delete a running exe.
* Clean up temporary measurement directories with `cargo clean --target-dir <dir>`.
