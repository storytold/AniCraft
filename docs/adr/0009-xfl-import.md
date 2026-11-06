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
* Next slices: bitmaps (needs bitmap fills, M5), text (M5), masks (M2), motion tweens, shape
  tweens, more scenes, and a corpus of larger self-authored files.
