# ADR 0010: Mask layers

* Status: accepted (2026-10-30)
* Relates to: roadmap M2 (mask layers) and M4 (XFL masks), ADR 0002 (render pipeline), ADR 0003
  (file format), ADR 0009 (XFL import)

## Context

Masks are one of the most used effects in 2D animation: a shape on one layer reveals only that
area of the layers under it (spotlights, windows, wipes, text reveals). Artists coming from
Animate expect the same model: a mask layer with one or more masked layers indented under it.
Imported Animate files use masks often.

## Decision

1. **Model.** `LayerKind::Mask`, and an optional `Layer::parent` (the mask a layer is masked
   by). This is the same parent link the XFL format uses, so layer folders can reuse it later.
   The invariant (a masked layer is a normal layer in an unbroken run directly under its mask)
   lives in one place, `Timeline::repair_masks`, run after layer edits, on load and on import.
   Invalid links are dropped, never trusted. Files stay v2: the field is optional and older
   files load unchanged.
2. **Evaluation stays pure.** `evaluate` emits clips into the display list: one per visible mask
   layer per timeline instance, nested in the enclosing clip (so masks inside symbols work).
   Masked items carry `clip`; the mask's own items carry `masks` and are never painted. A
   hidden mask masks nothing.
3. **Rendering.** A clip is the anti-aliased union of its mask fills, intersected with its
   parent clip. Strokes and opacity on mask artwork are ignored, as animators expect. Each
   render band computes a clip at most once, on first use. Clipped draws are bounded by their
   clip, so masks also cull. An empty mask hides everything under it.
4. **The mask always applies (what you see is what you export).** Animate shows the mask effect
   only while both layers are locked, which makes the stage disagree with the output. AniCraft
   always applies it, on the stage and in every export. To keep the mask editable, an unlocked
   mask layer shows its shapes' edges in the layer colour, and they can be picked and reshaped.
   Locking the mask hides the edge. Masked-away parts of shapes cannot be picked.
5. **Moving layers.** A layer moved in under a mask joins it, a layer moved out leaves it, and
   a mask layer moves with its whole group (stepping over a neighbouring group as a unit).

## Consequences

* The stage never disagrees with the export, and there is no lock-to-preview step to learn.
  Artists used to that step lose nothing: locking still works and also hides the edge.
* A future GPU renderer must implement the same clip semantics; the CPU tests are the oracle.
* Folders can reuse `parent` without a format change.

## Note (2026-11-12): soft masks

A mask layer now has a mask type (ADR 0070): Shape (this ADR's coverage, the default), Alpha or
Luminance. Soft masks use the same clips, nesting and stage edge; only how the artwork reveals
changes.
