# ADR 0005: Typography roles (prose vs data) and UI typefaces

* Status: accepted (2026-10-28)

## Context

* The project lead wants monospace type where the UI is "technical" (timeline ruler, seconds,
  readouts), as pro audio and video tools do, but not in dialog prose. We take the idea only,
  not any vendor's typeface.
* The fonts must be **permissive in use**, and either widely available as system fonts or small
  enough to bundle.

## Decision 1: semantic type roles, enforced

`theme::Type` defines ten roles. The family follows the *kind of text*: **data**
(numbers, timecode, rulers, measurements, shortcut keys, paths, preformatted text) is
monospace; **prose** (titles, headings, names, labels, captions, menus, buttons, dialog text)
is proportional. Mixed lines split label and value. Panels use `theme::text` /
`theme::font` / `theme::with_type` only; `guidelines_tests` fails the build on raw font
families or sizes outside `theme.rs`. Rules and examples: `docs/ui-guidelines.md`.

## Decision 2: typefaces

| Option | Licence | Footprint | Consistency | Verdict |
|---|---|---|---|---|
| System fonts (Segoe UI / Consolas, SF / SF Mono, DejaVu…) | Proprietary on Windows/macOS: may be *used* through the OS, but not bundled | 0 KB | Different metrics on every OS: monospace widths, layouts and screenshots change per machine | No: inconsistent; tests and docs can't be pixel-stable |
| egui defaults (Ubuntu Light + Hack, + Noto Emoji, emoji-icon-font) | UFL 1.0, MIT/Bitstream Vera, OFL, MIT: permissive in use | about 1.35 MB, already embedded | Same everywhere | **Fallback**: always present, guarantees glyph coverage |
| **Inter + JetBrains Mono** | SIL OFL 1.1 (free for any use, including commercial, embedding and redistribution; no reserved font names) | 402 KB + 264 KB ≈ 0.65 MB (can subset further) | Same everywhere; the PhotoCraft pair, so the Craft family looks consistent | **Primary** |

Both are among the most widely used open UI and code typefaces, and both are designed for
screens. JetBrains Mono has clear 0/O and 1/l/I distinctions, which suits frame numbers and
values.

## How they are delivered

As in PhotoCraft: **no font files are committed.** `build.rs` reads the optional
`CRAFT_FONTS_DIR` (a checkout of the shared craft-fonts repository) and embeds
`Inter-Regular.ttf` and `JetBrainsMono-Regular.ttf` when present. Official releases build with
it. Without it the app uses the egui defaults and still works (CI, Craft Anything checks,
contributors). Help › About shows which typefaces are active.

## Consequences

* Release packaging ships `OFL-Inter.txt` and `OFL-JetBrainsMono.txt` from craft-fonts next to
  the binary (`docs/legal/compliance.md`, release checklist).
* Footprint budget for embedded fonts: ≤ 2.5 MB total (today 2.0 MB with fallbacks). A later
  `slim-fonts` build can drop the egui text fonts once craft-fonts is mandatory for releases.
* Changing typefaces touches only `theme::font` and `build.rs`.
