# ADR 0011: Fixed-grid sprite sheet export

* Status: accepted (2026-11-06)

## Context

Game workflows need a PNG atlas and frame rectangles/timing. AniCraft already evaluates and
renders frames deterministically. A first exporter should reuse that pipeline without a
packing dependency or retaining every rendered frame in memory.

## Decision

`file.export_sprite_sheet` writes a PNG and sibling JSON through the command registry.
Frames occupy equal stage-sized cells, ordered left to right and then top to bottom. Columns
may be specified; otherwise choose the valid layout with the smallest maximum dimension,
breaking ties by area and then column count. No trimming, rotation, deduplication or padding.
Transparency is the default; including the stage background is optional. Unused cells remain
transparent. Frame ranges are inclusive, 0-based in engine params and JSON, 1-based in the UI
and CLI. JSON carries the source document fps and unrounded `1000 / fps` frame durations.

Use a small, explicitly versioned AniCraft JSON schema (documented in
[`sprite-sheets.md`](../sprite-sheets.md)); do not imply compatibility with a particular game
engine's atlas importer. Both outputs share a basename; the command requires a `.png` path.

Bound sheets to 8192 pixels per side, the renderer's 16,777,216-pixel surface budget, and
10,000 frames. Accumulate RGBA8 pixels while rendering one frame at a time. Desktop exports
run the command on a background session containing a persistent document snapshot. Editing
can continue, and only one sprite export runs at a time.

## Consequences

Either existing destination requires engine-enforced confirmation. Write both temporary
files before replacing either destination; remove temporary files on failure. Each file's
rename is atomic, but replacing the pair cannot be atomic across an OS interruption.

Large animations require a smaller scale or range. Artists get a size preview before export;
the desktop initially chooses a power-of-two scale reduction that fits the full timeline.
Packing and additional metadata adapters can be introduced later without changing v1's
rectangle and timing meanings.
