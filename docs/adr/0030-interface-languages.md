# ADR 0030: Interface languages and right to left

* Status: proposed (2026-11-07)
* Relates to: roadmap M8 (localisation), ADR 0004 (confirmations), ADR 0005 (typography), ADR 0008
  (visual language)

## Context

The interface was English only, and egui from crates.io lays Arabic out letter by letter from
the left, so a right-to-left language could not be shown at all. The other Crafting Apps share
their translations in storytold/craft-i18n, read them with craft-libs' `craft-i18n` crate, and
draw right-to-left text with storytold's egui fork.

## Decision

1. **English in the code, looked up at draw time** (`src/i18n.rs`): `tl!` and `i18n::t` for
   labels, `t_id` for command labels (by id), `trn` for counts with CLDR plurals, `fmt` for
   templates (values are isolated for mixed directions). The catalogs are data in craft-i18n
   (`locales/<code>/anicraft.tsv` beside the shared `common.tsv`), embedded by `build.rs` from
   `CRAFT_I18N_DIR`; without it the interface is English. Every embedded locale with an AniCraft
   catalog is offered, so a language is added in craft-i18n alone.
2. **What is never translated:** command ids, document text (layer, scene and symbol names),
   file names, the CLI, `serve` and MCP. Confirmations (ADR 0004) carry their message as a
   template with typed values (`confirm::Arg`) beside the English text, so the app translates
   them without reading English. Engine error messages stay English for now.
3. **Choosing:** Preferences › Interface language (Auto follows the system) and `--lang <code>`
   for one run. In debug builds `--lang pseudo` marks every string looked up, so text that skips
   translation shows in a screenshot.
4. **The egui fork:** storytold's egui and winit at the revisions the other apps use, through
   `[patch.crates-io]`. `deny.toml` allows exactly the storytold craft-libs, egui and winit
   repositories (MIT OR Apache-2.0), each pinned to an immutable revision. MSRV 1.95, for
   craft-libs.
5. **Right to left mirrors the window:** the tools sit on the right and the dock on the left;
   menus, rows, forms and dialogs start at the right (`theme::start_row`, `end_row`, `reading`,
   `column`); hand-painted widgets are laid out from the left and mirrored (`theme::mirror`).
   egui's `Grid` only runs left to right, so its cells are added in reverse.
6. **Time and artwork keep their direction:** the stage is never mirrored, and the timeline
   (frame axis, layer column, playback buttons and its header) stays left to right, as media
   timelines and transport controls do in right-to-left interfaces. Numbers, shortcuts, paths
   and unit symbols (px, pt, fps, %) are written as they are.
7. **Fonts:** Noto Sans Arabic (OFL 1.1, from craft-fonts) is the last fallback face of both
   families; another script adds its face the same way.

## Consequences

* New interface text goes through `tl!`, `i18n::fmt` or `i18n::trn`; a string that doesn't shows
  unmarked with `--lang pseudo`.
* Moving to a newer egui means a new pinned revision of the fork in `Cargo.toml`.
* CI builds the English interface until `craft-i18n.rev` names a craft-i18n commit with the
  AniCraft catalogs; then a missing checkout fails the build (`CRAFT_I18N_REQUIRED`).
