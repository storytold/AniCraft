# ADR 0078: The Swatches panel, swatches in the document, swatch files and GIMP palettes

* Status: accepted (2026-11-12)
* Relates to: ADR 0004 (destructive actions), ADR 0017 (plug-ins), ADR 0036 (files in the
  browser), ADR 0040 (interface languages), ADR 0064 (accessibility), ADR 0068 (brush sets as
  JSON), ADR 0074 (gradients and the fill picker), ADR 0075 (dockable panels), roadmap M1

## Context

Colour picking became one picker (#257, #261: web-safe swatches, mixer, hex, fill types and
gradient stops) and a dockable Colour panel (#265), but an artist could not keep a colour or a
gradient to use again: the guide said "no saved swatches yet". Animate keeps a **Swatches
panel** per document: a grid of colours and gradients, a default set for new documents, Add
Swatch, Duplicate, Delete, reordering, Clear Colors and Load Default Colors, and import and
export of swatch files. A click applies a swatch to the stroke or the fill, whichever is active
in the Tools panel and the Color panel.

Docking (#265) also left a regression: at the default size the timeline's layer footer cut
the **Delete** button to "Del…".

## Decision

### Model (`crates/doc/src/swatch.rs`)

1. **Swatches live in the document**, `Document::swatches: Arc<Vec<Swatch>>`, as Animate keeps
   them per document, so they are saved with the file, travel with it, and every change is an
   undo step like any other edit. They are not preferences.
2. **A swatch holds a paint without geometry**: a colour with alpha, or a linear or radial
   gradient's stops, overflow, Linear RGB and (radial) focal point. Where a gradient lies on a
   shape is not part of a swatch: applying one fits it to each shape, as a new gradient is
   fitted (ADR 0074). Bitmap fills are not swatches (Animate's aren't either); adding one says so.
3. **Older files and the default set.** The field is `#[serde(default = default_set)]` and is
   skipped while it equals the default set. So every file written before this ADR opens with
   the default swatches and is written back unchanged until the swatches change, and an
   explicitly emptied list stays empty (`[]` is not the default set). `Document::sanitize`
   repairs loaded swatches (colours clamped, stops ordered and bounded to 2–15, focal point
   held to ±0.98, names one line of at most 128 characters) and drops ones it can't repair; at
   most 1,024 are kept.
4. **The default set is our own**: greys from black to white, the primaries and secondaries
   (12 colours) and seven gradients (black to white, white to black radial, red, green and blue
   balls, a spectrum and gold). Animate's default panel repeats the 216 web-safe colours; the
   picker already shows that standard cube, so the document's set stays short enough to read
   as one row in the picker.

### Commands (`crates/engine/src/cmds_swatch.rs`)

`swatch.list`, `swatch.add` (the current fill or stroke, or a swatch given as JSON, at an
index), `swatch.remove`, `swatch.move`, `swatch.duplicate`, `swatch.rename`, `swatch.apply`
(`target`: fill or stroke; `selection`: also the selected shapes), `swatch.clear` (keeps black
and white, as Animate's Clear Colors), `swatch.reset` (the default set), `swatch.import` and
`swatch.export`. Everything the panel does is one of these, so it works the same from the CLI,
MCP and scripts.

* **Applying reuses the existing paths**: a fill is `paint.set_fill` (sets the fill swatch and
  the selected fills in one step, fitting a gradient to each shape); a stroke is
  `element.set_stroke` on the selection plus the stroke colour. A gradient can't be a stroke
  (strokes are one colour), so `target: stroke` with a gradient is an error, and the panel
  sends gradients to the fill whichever colour is active.
* **ADR 0004 tiers.** Removing one swatch is a routine, undoable edit. Clear, Load Default
  Swatches and an import that replaces the swatches confirm by default (undoable, so not
  `irreversible`); exporting over an existing file always confirms.
* **Plug-ins** (ADR 0017) may read and edit the document's swatches, as any document data,
  but `swatch.import` and `swatch.export` read and write files and are refused.

### Swatch files

* **Our format** is a small JSON file, `{"format": "anicraft-swatches", "version": 1,
  "swatches": [{"type": "solid", "color": "#rrggbbaa", "name": …}, {"type": "linear", "stops":
  […], "spread": …, "linear_rgb": …}, …]}`, the same shape `swatch.add` takes, so one parser
  serves both. Unknown swatch kinds in a newer file are skipped rather than failing the import,
  as brush sets do (ADR 0068). Files are bounded to 1 MiB and 1,024 swatches.
* **GIMP palettes** (`.gpl`) import: a plain-text open format (a `GIMP Palette` line, optional
  `Name:`/`Columns:` lines and comments, then `R G B name` lines), documented by the GIMP
  project. GIMP's "Untitled" names show as the hex instead. Import tells the two formats apart
  by content, not by extension.
* **Not read: Adobe's `.aco`, `.ase` and `.clr`.** They are Adobe's proprietary formats with no
  open specification we may implement from; we skip them (clean room, AGENTS.md rule 6).
* **XFL import of a document's swatches: not yet.** The public descriptions of XFL we work from
  (ADR 0009) don't describe how a document's swatches are stored, and our self-authored corpus
  (ADR 0035) has no sample to observe. Animate files open with the default set.

### Shell

* **Window › Swatches** is a dockable panel (ADR 0075): docked beside the Colour panel when
  that is open (the Designer workspace tabs Colour, Swatches and Align), floating like any other.
  A grid of swatches wrapped to the panel's width: a solid swatch shows its colour (a
  checkerboard under alpha), a linear gradient its ramp, a radial one its rings. Hover shows the
  name and hex (or "Linear gradient, 3 colours"). Click applies; drag moves (a bar shows where
  it lands); right-click offers Apply to Fill, Apply to Stroke, Duplicate, Delete and the name.
  The bar below adds the active colour (+), duplicates and deletes the picked swatch; its menu
  (…) imports, exports, loads the defaults and clears.
* **Stroke or fill is one shared choice**, `App::colour_target` (view state): set by clicking
  the Tools panel's Stroke or Fill square or the Colour panel's, or the "Swatches set the
  Stroke / Fill" toggle shown in both the Colour and the Swatches panels. A new user who never
  touches it gets Fill.
* **The pickers show the document's swatches**: the colour picker's Swatches view has a
  "Document swatches" row (solid colours, three rows at most, names on hover) under the recent
  colours, and the fill picker has a row of the document's gradient swatches under its type row.
  The app hands the list to the pickers each frame (an `Arc` clone), so the pickers stay
  independent of `App`.
* **Accessibility**: every swatch is its own button, named "Swatch 3: Teal (#2a9d8f)", selected
  state exposed; Enter applies, Delete removes, Alt+Left/Right moves the focused swatch, so
  reordering doesn't need a drag. The bar's icon buttons are named by their tooltips.
* **The layer footer** is icon buttons only, as the timeline header became in #256: New Layer
  (+) and Delete (trash) on the left as in Animate, New Folder, Up and Down on the right, each
  named by its tooltip. Nothing in it is text, so it fits at any window size and in any
  language (tested at the default size; checked with `--lang pseudo`). The Plus, Trash, Copy
  and Ellipsis icons are Lucide's (ISC), copied unchanged.

## Consequences

* A document carries its palette; teams share one by exporting a swatch file, or by starting
  from a template document (ADR 0065) that has it.
* Swatch edits sit in the undo history next to drawing, as in Animate.
* Files grow only when the swatches differ from the default set (a few hundred bytes for a
  typical palette).

## Not in this slice

* Swatch folders or groups, and Animate's "Sort by Color".
* Bitmap swatches; gradient strokes (a gradient swatch always fills).
* Saving the panel's swatches as the default for new documents (Animate's Save as Default).
* Reading Adobe `.aco`/`.ase`/`.clr` (proprietary) and XFL swatch import (no public description).
