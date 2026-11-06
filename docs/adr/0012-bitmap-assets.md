# ADR 0012: Shared bitmap assets and native format v3

* Status: accepted (2026-11-06)

## Context

M5 bitmap import needs portable projects and shared pixels across instances, frames and undo.
Stage, picking, timeline evaluation and exports must continue to use one evaluated document.

## Decision

* Add `anicraft-bitmap` at L0: immutable validated RGBA8 pixels, bounded PNG/JPEG decoding and
  PNG serialization. Use `png` and `zune-jpeg` (SIMD features and unsafe decoding disabled).
  Apply JPEG EXIF orientation on import; treat input colours as sRGB without ICC conversion.
* A `SymbolKind::Bitmap` Library entry holds a rectangular shape with `Paint::Bitmap`, an
  `Arc<Bitmap>` and an image-to-local transform. Instances use existing symbol evaluation,
  transforms and tweens. Break Apart retains the bitmap fill. No separate image evaluator.
* Sample bilinearly in premultiplied space through the existing coverage/mask compositor.
  Images do not repeat. SVG embeds PNGs with baked colour effects, reuses identical
  image/effect definitions and clips image domains to prevent repeating SVG patterns.
* Native format v3 adds a `bitmaps` table before `document`: one base64 lossless PNG per shared
  image, referenced by index from paints. Restore `Arc` sharing on load. Keep the JSON envelope
  and path table; continue reading v1/v2. Older releases refuse v3 instead of dropping artwork.
  A zip container remains a future format decision.
* Bound images to 16,777,216 pixels, 16,384 px per side and 80 MiB input. Bound document pixels
  to 256 MiB / 4096 allocations during native loading, import and clipboard merges. Identical
  standalone imports share pixels. Undo charges pixel resources once and recognizes shared
  Library assets without scanning every frame during instance edits.
* `file.import_bitmap` adds the asset and optional stage instance as one command/undo step.
  Desktop decoding runs on a worker; its prepared result goes through the same command.
  Document replacement cancels a pending import; otherwise place at the current layer/frame.
* Remap clipboard symbol graphs across documents, avoiding collisions with unrelated assets
  that have the same source ids.

## Consequences

Documents remain usable after the original image is deleted or moved. New files require a
v3-capable reader. XFL bitmap instances backed by standard PNG/JPEG package assets import;
proprietary payloads and XFL bitmap fills remain reported. Image sequences, relinking, pixel
editing, ICC conversion and bitmap-fill controls are outside this slice.
