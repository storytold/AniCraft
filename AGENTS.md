# AGENTS.md: guide for AI agents and contributors

AniCraft is an open-source, native, clean-room 2D animation studio (Adobe Animate class), written
in **Rust only**. There is no Tauri, Electron or webview. The UI is egui/eframe. The product name
is written **AniCraft**; machine names are lowercase (`anicraft-*`). Read `docs/` first.

## Workspace map

```text
crates/geom     L0  Vec2, Affine (decompose/compose), Rect, Path, flattening, hit testing
crates/bitmap   L0  Immutable RGBA assets, bounded PNG/JPEG decoding and PNG serialization
crates/doc      L1  Document model, easing, timeline evaluation, sample scene
crates/render   L2  CPU oracle renderer (coverage accumulation, paints, strokes, PNG)
crates/xfl      L2  XFL/FLA import: own XML + zip readers, edges → paths (ADR 0009)
crates/engine   L3  Session, undo (with gesture coalescing), command registry, .anicraft format
crates/cli      L4  anicraft-cli: info/render/render-seq/commands/exec/run/new/serve
src/            L5  egui shell (app, menus, stage, timeline, panels, theme)
```

A crate depends only on lower layers. Nothing below `src/` may use egui, eframe or winit.

## Standing rules (read `docs/product-principles.md`)

* **Principal-engineer check.** For every proposal ask: "Is this how a principal engineer
  would do it?" If not, use the better method or ask the project lead. Record hard-to-reverse
  decisions as ADRs in `docs/adr/`.
* **Users are artists and creatives** (many coming from Adobe Animate). The product must be
  professional, fast, reliable and easy, with smooth, predictable flow: common actions one
  click or key away, shortcuts for power users, no modal interruptions for routine edits.
* **UI guidelines** (`docs/ui-guidelines.md`, ADR 0005): text goes through `theme::Type`
  roles. Data (numbers, timecode, rulers, values, shortcuts, paths) is monospace; prose
  (labels, menus, buttons, names, dialog text) is proportional. No raw font sizes or colours
  outside `theme.rs` (enforced by `guidelines_tests`). Typefaces: Inter + JetBrains Mono via
  `CRAFT_FONTS_DIR`, with egui defaults as fallback.
* **Cross-platform** (ADR 0006): macOS, Windows and Linux are first-class. OS-specific
  behaviour lives only in `src/platform.rs` (shortcut labels with ⌘ on macOS, app menu,
  window chrome, settings folder, native dialogs, GPU backend fallback). Shortcut text always
  goes through `platform::shortcut_label`.
* **Edits are live**: anything that changes the document must show in the same frame (Auto
  render mode). Never leave an edit waiting for the next input event.
* **Destructive actions** follow the tiers in ADR 0004: irreversible loss always confirms
  (engine-enforced `"confirm": true`); large undoable deletions confirm by default; routine
  edits rely on undo with a visible hint.

## Golden rules

1. **Never crash.** Non-test code never panics: no `unwrap`/`expect`/`panic!`/indexing on
   input-derived values. Crates carry `#![deny(clippy::unwrap_used, …)]`. Commands validate params
   and return `Err`. The `panic_hunt_every_command` test fuzzes every command and must stay green.
2. **Everything is a command.** New behaviour means a `CommandSpec` in `crates/engine/src/cmds_*.rs`
   with id, label, menu path, shortcut, params doc and tests. Menus and shortcuts are generated
   from the registry. Only view state (zoom, pan, tool, onion skin, playback) lives in the shell.
3. **Time is a function.** Never animate by mutating the document. Add behaviour to
   `doc::evaluate` and keep it pure.
4. **The CPU renderer is the oracle.** Renderer changes come with analytic or golden tests. Any
   future GPU renderer is tested against it.
5. **No unsafe** (`unsafe_code = "forbid"`).
6. **Clean room.** Learn behaviour from public docs and observation only. Never copy proprietary
   code, presets, shaders or assets.
7. **Verify UI changes visually**: `cargo run -- --screenshot shots/now.png`, then look at the PNG.

## Licensing and legal compliance (read `docs/legal/compliance.md`)

* AniCraft is **MIT OR Apache-2.0**, like PhotoCraft (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`).
* **Dependencies**: permissive only, from crates.io only. `deny.toml` is the policy, and
  `cargo deny check licenses bans sources` must pass. Adding a crate with a new licence needs
  a deliberate `deny.toml` change and a note in `compliance.md`. No copyleft, "non-commercial"
  or source-available terms.
* **Assets**: every font, icon, image, sound or sample file gets a row in `ATTRIBUTION.md`
  in the same change, with its licence text under `licenses/`. Never commit font files
  (use the shared craft-fonts input). Never commit Adobe or other proprietary material.
* **Patents**: implement only from public papers, open specifications and permissively
  licensed references; credit them in `ATTRIBUTION.md`. Never bundle encoders for
  patent-pool codecs (H.264/HEVC/AAC); use royalty-free formats or a user-installed ffmpeg.
* **Trademarks**: Adobe, Animate and Flash only nominatively (docs, and the non-affiliation
  disclaimer in legal notices), never in the product name, branding, feature UI or marketing.

## Version control

* Git. Remote `origin` = `https://github.com/storytold/AniCraft` (public). `main` is the
  development branch; it begins with "Initial commit v0.1.0 early alpha", and contributors are
  credited in `AUTHORS.md`.
* **Every change reaches `main` through a pull request** from a topic branch (`feat/…`, `fix/…`,
  `docs/…`), with one roadmap step per PR, green on build, tests, clippy and `cargo deny`.
  Never commit or push to `main` directly. `core.hooksPath` points at `scripts/git-hooks`:
  `pre-commit` refuses commits on `main`, and `pre-push` refuses pushes to the remote `main`.
  Run `git config core.hooksPath scripts/git-hooks` once per clone.
* Branches marked hidden in your local git config (`git config --add anicraft.hidden <branch>`)
  never leave the machine: `pre-push` refuses them and anything built on them.
* Commit in small, green steps (build + tests pass), with conventional messages:
  `feat(render): …`, `fix(engine): …`, `docs: …`, `perf: …`, `test: …`. Record measured
  numbers in `perf:` commit bodies.
* Never commit `target/`, screenshots (`shots/`), exports or `.anicraft` user files.

## Build hygiene (disk first, then time; `docs/development.md` › Build footprint)

* Use the **single shared `target/`**. No per-agent `CARGO_TARGET_DIR` copies (each is a
  full build).
* Don't turn incremental compilation or full debug info back on in `Cargo.toml`. Do that
  locally if you need a debugger.
* After profile or feature changes, run `cargo clean` once (with the app closed).

## Before you finish

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo deny check licenses bans sources
cargo run -- --screenshot shots/now.png
```

Update `docs/roadmap.md` › Honest status when you move a capability.
