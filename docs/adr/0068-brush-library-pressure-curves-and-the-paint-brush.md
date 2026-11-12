# ADR 0068: Brush library, pressure curves and the Paint Brush

* Status: accepted (2026-11-11)
* Relates to: ADR 0033 (Brush tool), ADR 0034 (Eraser), ADR 0018 (variable-width strokes),
  ADR 0004 (destructive actions), ADR 0059 (recent files: session data kept in preferences),
  ADR 0017 (plug-in policy)

## Context

ADR 0033's Brush paints the exact sweep of a round or square tip, with pen pressure taking it
from 10 % to 100 % of its size. Artists coming from Animate expect more. They want a library of
brushes to pick from, brushes of their own saved under a name and moved between computers, a
pressure curve to fit their pen and hand, and the **Paint Brush (Y)**. The Paint Brush draws a
variable-width *stroke* in the stroke colour, not a fill, and the Width tool reshapes it later.
We learned this behaviour from Adobe's public help pages and by watching the program. Nothing
was copied: no presets, no shapes and no art brushes.

## Decision

1. **A brush is parameters, never pixels.** A preset (`anicraft_engine::brush_library::BrushPreset`)
   holds:
   * the tip: size, round or square, angle, roundness;
   * smoothing;
   * a pressure curve, and what pressure drives (size, opacity);
   * shaping: a taper at the start and at the end (percent of the stroke's length), dab spacing
     (0 for a continuous sweep, or 5–1000 % of the size between dabs), and a ragged edge (0–50 %
     of the size).

   No bitmaps, so a preset is a few numbers, renders crisply at any size, exports as vectors
   everywhere, and the same points and settings always give the same outline (tested for every
   preset in both modes). A preset's defaults are ADR 0033's plain brush, so `draw.brush` without
   the new params does exactly what it did.

2. **Shaping is in the outline, pure geometry** (`anicraft_geom::brush::Shaping`, `outline_shaped`):
   * **Tapers** multiply the tip's size factor by an ease-out ramp over the first and last part
     of the length (down to 5 % of the size). The centre line is cut finely enough to follow the
     ramp: a straight drag flattens to two points.
   * **Dabs** stamp the tip alone at even arc-length steps along the smoothed centre line. There
     are at most 20 000 per stroke; past that, the spacing widens.
   * **The ragged edge** lowers the distance field by `wear × grain(x)`. The grain is a fixed
     two-octave value noise in tip space, hashed with SplitMix64's finaliser. Because the field
     is a true distance, the edge moves in by exactly that much. The inside reaches deeper by
     the same amount, so the wear never opens holes, and no orientation or self-intersection
     cleanup is needed. Cells get finer (1/14 of the radius) to resolve the grain. The pattern
     is anchored in tip space, not per stroke, so the result is deterministic.

   *Rejected:* displacing the traced outline along its normals. It needs each loop's
   orientation and can fold over itself in tight corners.

3. **Pressure curves** (`anicraft_geom::pressure::PressureCurve`):
   * Up to 8 points with ends at input 0 and 1, interpolated by a monotone cubic
     (Fritsch–Butland tangents, Fritsch–Carlson limits). The curve is smooth, passes through its
     points, never overshoots, and stays in 0..1. It is `Copy`, so tool options stay cheap.
   * Presets: Linear; Soft (a light touch paints wide); Firm (it takes a firm press).
   * The curve maps the measured pressure before it sizes the tip.
   * **Opacity** from pressure is the stroke's average mapped pressure times the colour's alpha.
     A brush stroke is one solid fill, and opacity that varies along a stroke needs paint along
     a path (not yet). A mouse paints opaque.

4. **The Paint Brush is `draw.brush` with `as_stroke`.**
   * The smoothed centre line becomes an open path. It gets a stroke of the brush's size in the
     stroke colour.
   * The pressure and taper factors are sampled at 256 places, simplified by Ramer–Douglas–Peucker
     (within 1.5 % of the size; the tolerance widens until at most 64 points, the profile limit),
     and stored as an ADR 0018 width profile. The Width tool then edits it like any other stroke.
   * A stroke of one width has no profile, so it is a plain line that merges like the Pencil's
     (ADR 0031 merges only uniform strokes).
   * As a stroke, the tip's shape, angle and roundness, dabs and the ragged edge don't apply: a
     width profile is symmetric across the line. In the shell it is its own tool (Y, after the
     Brush) with its own options, as in Animate.

5. **User brushes belong to the session, like recent files (ADR 0059).** `Session::brushes`
   survives New and Open. The desktop app copies it into its preferences when a command changes
   it, and puts it back at start. The engine never reads the settings folder, and the commands
   work the same from the CLI and MCP.
   * Names are unique (case doesn't matter), one line, at most 64 characters, and never a
     built-in's name. There are at most 200.
   * **A brush set** is a small JSON file (`{"format": "anicraft-brushes", "version": 1,
     "brushes": [..]}`). Import skips bad entries, clamps values, numbers clashing names ("Ink
     2"), and reads at most 1 MB.

6. **Commands.**
   * `brush.preset_list`, `brush.preset_save` (`from` a preset, with any settings),
     `brush.preset_rename`, `brush.preset_delete`, `brush.preset_export` (yours, or `names`) and
     `brush.preset_import`.
   * `draw.brush` gains `preset`, `curve`, `pressure_size`, `pressure_opacity`, `taper_start`,
     `taper_end`, `spacing`, `jitter` and `as_stroke`. Explicit params override the preset's.
   * ADR 0004: deleting a brush and exporting over a file can't be undone, so both are
     engine-enforced confirmations. Saving and renaming don't touch the document.
   * Plug-ins may list brushes but not change them or touch brush set files.

7. **Built-in brushes are AniCraft's own**: Pencil, Ink Pen, Flat Calligraphy, Round Taper, Marker,
   Chalk and Dotted, each a set of these parameters (`brush_library::builtin`).

8. **Shell.**
   * **Window › Brush Library** lists every brush with a stroke preview drawn by the CPU renderer.
     The previews come from a scratch session and are cached as textures, keyed by the settings,
     colour and pixel size. A click sets the brush in hand: the Paint Brush when it is in hand,
     else the Brush.
   * Your own brushes are listed below the built-ins, with Rename and Delete on right-click. The
     window also has Save Brush, Import… and Export…, through the native file dialogs (the
     browser's picker on the web).
   * Properties shows a Brush menu, a live preview of the current settings, Pressure (Size,
     Opacity), and the **curve editor**. In the editor, drag a point to move it (the ends move
     only up and down), drag elsewhere to add one, and double- or right-click a point to remove
     it. Linear, Soft and Firm sit beside it.
   * Properties also has **Velocity** ("Speed as pressure", off by default) and the tapers; the
     Brush adds Spacing and Rough edge.
   * Velocity eases the pressure toward `1 − 0.8 × min(speed / 2400 pt/s, 1)`, from half
     pressure at the start, only when no pen reports pressure. Its pressures go into the points,
     so the command stays deterministic and headless.

9. **Tilt is not in this slice.** egui 0.36 reports only `force` for pens, and winit 0.30 gives
   stylus altitude only on iOS. With no tilt to read on any desktop system, mapping tilt to the
   tip's angle waits for the toolkit. A per-sample angle would also change the sweep's distance
   field, which assumes one tip transform.

## Consequences

* Measured in release, 32 threads, on a 400-unit wave with pressure 0.3 → 1 → 0.3 (median of
  15 runs, whole command):

  | Preset | Fill (anchors) | As a stroke |
  | --- | --- | --- |
  | Pencil | 22.2 ms (36) | 0.04 ms |
  | Ink Pen | 36.3 ms (26) | 0.04 ms |
  | Flat Calligraphy | 3.1 ms (18) | 0.03 ms |
  | Round Taper | 64.9 ms (21) | 0.04 ms |
  | Marker | 1.7 ms (25) | 0.03 ms |
  | Chalk | 3.0 ms (321) | 0.03 ms |
  | Dotted | 0.6 ms (76) | 0.03 ms |

  Tapers cost the most: their 5 % tips need fine cells. That cost comes once, when the stroke
  ends. A 300 × 80 px library preview of Chalk renders in 1.5 ms. Reproduce with `cargo test
  --release -p anicraft-engine preset_timings -- --ignored --nocapture`.

* Each preset's area on that wave stays within tested bounds (as a part of length × size).
  Dotted is exactly π/8.8 of it.
* The live preview while dragging is still the raw sweep of the plain tip. It doesn't show
  dabs, the ragged edge or (for the Paint Brush) the smoothing.
* Paint Brush strokes with pressure are drawing objects, even with Object Drawing off, because
  width-profiled strokes don't merge yet (ADR 0018, 0031).

## Not yet

* Tilt.
* Opacity that varies along a stroke.
* Textured, bitmap, art and pattern brushes.
* Custom tip shapes.
* Pressure for the Eraser.
* Brush presets for the Eraser.
* A live preview through the shaping.
* Sharing brushes inside a document.
* Animate's brush library import (its format is not public).
