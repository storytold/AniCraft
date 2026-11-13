# ADR 0009: XFL / FLA import

* Status: accepted (2026-10-29)
* Relates to: roadmap M4 (the project lead's first interchange priority), ADR 0003 (file format)

## Context

Artists coming to AniCraft have their work in Animate documents: `.fla` files (a zip archive)
or `.xfl` packages (the same content as a folder). Opening them is the first interchange
priority. The format is an XML package: `DOMDocument.xml` (stage, scenes, layers, frames,
elements) and `LIBRARY/*.xml` (symbols), with shape geometry encoded as edge strings.

## Decision

1. **A separate crate, `anicraft-xfl`**, between `doc` and `engine`. It depends only on `doc`,
   `geom` and `miniz_oxide` (already in the tree through `png`), so the parser is isolated,
   fuzzable and testable on its own, and the engine just calls it.
2. **Own XML and zip readers.** No XML or zip crate is available offline, and XFL needs little:
   elements and attributes (geometry lives in attributes), and stored/deflate zip entries. Both
   readers are small, bounded (size, depth and element-count limits; zip-bomb caps) and never
   panic. A full XML crate can replace ours later without touching the mapping.
3. **Clean room.** Written from the publicly documented structure of XFL and the published ZIP
   specification (PKWARE APPNOTE). Shape fills are rebuilt from the edges' left/right fill
   styles by chaining segments into closed loops. No Animate code, no third-party converter
   code. The test corpus is self-authored.
4. **One entry point: `file.open`.** It detects `.fla`, `.xfl` and XFL folders, so menus, the
   palette, the command line and agents all import the same way, and it inherits the
   unsaved-work confirmation. **An imported document is untitled**: Save asks for a new
   `.anicraft` name and can never write over the Animate file.
5. **Report, don't guess.** Anything not yet mapped (bitmaps, text, sound, masks, motion
   tweens with property curves, shape tweens, filters, extra scenes) is counted and shown in
   the status bar after opening ("Imported. Not yet supported: 2 bitmaps, 1 text field.").

## What maps in this first slice

Stage size, frame rate and colour; the first scene; layers (name, colour, visible, locked,
outline, guide; folders flattened); keyframes with durations and labels; classic tweens with
their ease; shapes with solid, linear and radial fills and solid strokes (other stroke styles
drawn solid); groups (flattened); rectangle and oval primitives; symbol instances with matrix,
alpha, tint, loop mode and first frame; library symbols (graphic, movie clip, button) with their
own timelines, nested to any depth.

## Consequences

* Imported documents render and animate in AniCraft immediately and can be edited with every
  tool, then saved as `.anicraft`.
* Text fields come across as text blocks (ADR 0014: one block per run of same-style lines,
  missing fonts named in the report).
* Next slices: bitmaps (needs bitmap fills, M5), masks (M2), motion tweens, shape
  tweens, more scenes, and a corpus of larger self-authored files.

## Testing

* Unit tests on inline snippets in `crates/xfl/src/tests.rs`, `text_tests.rs` and
  `edges.rs` pin single behaviours (edge decoding, fill rebuilding, each mapping).
* **The corpus** (ADR 0035): twelve self-authored XFL packages in `crates/xfl/tests/corpus/`
  that together use every feature the importer claims. Each has an exact report expectation,
  structural checks and about 270 hostile variants (cut short, garbled, files missing) that
  must never panic (`crates/xfl/tests/corpus.rs`), and golden renders of chosen frames by the
  CPU oracle with hand-worked colour probes (`crates/engine/tests/xfl_corpus.rs`, regenerated
  with `UPDATE_GOLDENS=1`). A feature the importer learns gets a corpus file in the same change.
* **`.fla` archives** (ADR 0054): every corpus package, packed by an independent in-memory zip
  writer in nine layouts (Animate's `mimetype` first, stored, mixed with folder entries,
  Windows separators, CP437 and Unicode-path names, data descriptors, ZIP64, all at once),
  imports identically to its folder and renders the same goldens; a fuzz of 9,664 hostile
  archives built from the corpus never panics, and the zip reader refuses overlapping entries,
  lying sizes and checksums, ratio bombs, unsafe and duplicate paths and deep nesting with a
  readable error (`crates/xfl/src/zip_tests.rs`, `crates/xfl/tests/fla.rs`).
