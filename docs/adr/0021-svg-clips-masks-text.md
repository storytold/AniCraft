# ADR 0021: Clip paths, masks and text from SVG drawings

* Status: accepted (2026-11-09)
* Relates to: roadmap M4 (SVG import), ADR 0010 (mask layers), ADR 0012 (text), ADR 0014
  (text from Animate documents)

## Context

Import to Stage (`file.import`, `crates/svg`) brought an SVG drawing across as shapes on one
layer, and reported `clipPath`, `mask` and `text` as skipped (drawn unclipped, or not at
all). Those three are everywhere in real drawings: design tools wrap every frame in a clip,
illustrations crop artwork into shapes, icons use masks, and posters and diagrams carry text.
AniCraft already has the models to receive them: mask layers (ADR 0010: a mask layer's fills
reveal the normal layers under it) and text blocks (ADR 0012), with the font lookup and
reporting written for Animate documents (ADR 0014). AniCraft's own SVG export writes masks
as `clipPath`s, so a masked frame should also come back.

SVG clipping is more general than mask layers: clips nest (a clipped group inside a clipped
group), apply to single shapes, and a `mask` is a soft luminance (or alpha) image, not an
area. The behaviour here comes from the W3C SVG 1.1 / SVG 2 and CSS Masking specifications
and from self-authored drawings.

## Decision

1. **A clipped drawing becomes a folder of layers.** The importer reads the drawing in
   order and every drawn item collects the clip regions of the elements around it. At the end
   consecutive items with the same effective clip form one run; a run with a clip is a masked
   layer under a mask layer holding the clip's shapes. Import to Stage puts the runs, top
   first, in a folder named after the file (inserted where a new layer would go, above the
   active mask group if the active layer is masked). A drawing with no clips still makes one
   layer named after the file, as before. Mask layers come in locked, so the stage shows the
   clean result; unlocking shows their edge as for any mask (ADR 0010). One undo step.
2. **Resolve clips exactly, or say so.** A clip is the union of its shapes' fills
   (`clip-rule`, transforms, `use`, `clipPathUnits`, a `clip-path` on the `clipPath` itself).
   For each item: a convex clip that contains the item cuts nothing and is dropped (the frame
   clip of design-tool exports), a clip that misses it hides it (left out). Several clips left
   are intersected exactly when all but one are convex polygons, by clipping the remaining
   clip's shapes against the polygons (Sutherland–Hodgman extended to Bézier segments,
   `anicraft_geom::clip`: curves stay curves, holes and fill rules survive). Two curved clips
   can't be intersected without general path booleans: the inner one is kept and the report
   counts it. Mask layers ignore strokes and opacity, which matches clip paths exactly.
3. **Masks come across when they are coverage.** A `mask` whose content is opaque (fill alpha
   and opacity 1, no strokes, no clips inside, and white for the default luminance masks; any
   colour for `mask-type: alpha`) equals the area of its fills, cut to the mask's region
   (`maskUnits`, `x`/`y`/`width`/`height`, default the bounding box grown by 10%), so it
   becomes a mask layer the same way. Any other mask is soft: the artwork is drawn unmasked and
   the report counts "soft, faded or coloured masks". Soft masks need per-pixel mask values in
   the renderer; they wait for that.
4. **Text becomes text blocks, one per text chunk.** A chunk starts at a `text` and at every
   `tspan` with an absolute `x`; it is one single-line block placed by its baseline (`x`, `y`,
   a leading `dx`/`dy`; `anicraft-text` now reports the first baseline) and `text-anchor`
   (start, middle, end, also kept as the block's alignment). Style: `font-family` (the first
   installed family of the list, generic families mapped to `_sans`, `_serif`,
   `_typewriter`), `font-size` (lengths, keywords, `em`/`%`), `font-weight` (600 and up is
   bold), `font-style`, `letter-spacing`, `fill` and `fill-opacity`, and `opacity`.
   Whitespace collapses as in browsers unless `xml:space="preserve"`. A chunk whose pieces
   differ in style takes the style of most of its visible characters and is counted, as for
   Animate text (ADR 0014). With no `font-family` the text is serif, as browsers draw it.
   When no family of the list is installed, the text falls back to the default family and the
   report names the first one ("Fonts not on this computer, drawn in Inter: Futura."). Text
   inside a `clipPath` stays a text block on the mask layer, so the mask stays editable.
5. **Report, don't guess.** Per-character positions (lists of `x`, `y`, `dx`, `dy`; a new
   `y` mid-line), `rotate`, `textPath`, text decoration, vertical writing modes, strokes and
   gradients on text, CSS clip shapes and mask images, clips on a clip's contents, clip loops,
   and more than 250 masks in one drawing (the rest drawn unclipped) are counted. Masks draw
   their content again, so the work is bounded by a count of visited elements (2 million), not
   by the file size.
6. **Ordered text needs mixed content.** The shared XML reader keeps character data in order
   among the children of elements named by the caller (`anicraft_xml::parse_mixed`, as
   `#text` nodes); other elements, and XFL, read as before.

## Consequences

* Masked frames exported as SVG come back as the same picture on mask layers (tested: pixel
  identical at half size), and typical design-tool exports lose their redundant frame clips.
* Imported clips are editable as AniCraft masks: move, reshape or unlock them like any mask.
* A drawing with many clips makes many layers (two per masked run), inside one folder.
* Text in SVG files is laid out with this computer's fonts once, on import, then stored as
  outlines like all text (ADR 0012).
* Not yet: soft (luminance and alpha) masks, booleans between two curved clips, per-character
  positions and rotation, text on a path, multi-line blocks from stacked chunks, dashes
  (another track owns strokes), images and style sheets.

## Note (2026-11-12): soft masks, text paths, per-character positions

ADR 0070 imports soft masks onto soft mask layers (luminance by default, `mask-type: alpha`),
`textPath` onto text on a path, and per-character `x`, `y`, `dx`, `dy` and `rotate` where they
are simple (a block per positioned character).
