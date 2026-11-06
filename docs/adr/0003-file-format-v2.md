# ADR 0003: `.anicraft` v2: SVG path strings, shared path table, streaming read

* Status: accepted (2026-10-28)

## Context

Measuring before optimizing paid off here. The v1 writer turned an 86 MB in-memory stress
document into **782 MB** of JSON (925 ms to save, 633 ms to load). Two causes: paths shared in
memory across keyframes were repeated in every keyframe, and paths were written as verbose
tagged JSON.

## Decision

* `Path` serializes as compact SVG path data (`"M0 0L10 0C…Z"`) with shortest round-trip
  floats. The parser is strict, bounded and never panics.
* Shared paths are written once in a top-level `paths` table; shapes store an index.
  `path_table::build_table` walks the document first, so the table precedes the document in the
  file. The reader streams in one pass and resolves references to shared `Arc`s, which
  **restores in-memory sharing on load**.
* Small files (< 1 MB) are pretty-printed for humans and diffs; large files are compact.
* v1 files (inline legacy paths) still load; newer-than-supported versions are refused.

## Measured (same stress document)

| | v1 | v2 |
|---|---|---|
| File size | 782 MB | 65 MB |
| Save | 925 ms | 118 ms |
| Load | 633 ms | 142 ms |

## Next

A zip container for embedded media (M2); optional binary encoding if profiling shows JSON
parsing dominates on real XFL-imported files.
