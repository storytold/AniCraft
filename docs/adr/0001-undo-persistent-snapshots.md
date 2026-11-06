# ADR 0001: Undo/redo on persistent (structurally shared) document snapshots

* Status: accepted (2026-10-28)
* Supersedes: the M0 full-document-clone history

## Context

AniCraft must handle large files: Animate projects with hundreds of layers, thousands of
keyframes and hundreds of thousands of element instances once XFL import lands. Undo has to be
exact (it is the user's safety net), memory-bounded, cheap per edit, and it must not slow down
interactive gestures such as dragging.

## Options considered

| Option | Exactness | Per-edit cost | Engineering risk |
|---|---|---|---|
| A. Full-document clone per step (M0) | exact by construction | O(document) time and memory | none, but unusable at scale |
| B. Command pattern (each command records its inverse) | only as good as every hand-written inverse | O(change) | high: dozens of commands, each needing a correct inverse, and every new command adds risk |
| C. Diff/patch of serialized states | exact | O(document) to diff | medium; slow on big files |
| **D. Persistent document + snapshot per step** | exact by construction | O(path to the change) | low: one rule (mutate through `Arc::make_mut`) |

## Decision

**D.** The document is a persistent tree: `Timeline.layers: Vec<Arc<Layer>>`,
`Layer.keyframes: Vec<Arc<Keyframe>>`, `Keyframe.elements: Vec<Arc<Element>>`,
`Shape.path: Arc<Path>`, `library: Vec<Arc<Symbol>>`. Mutation goes through `Arc::make_mut`,
which copies only the nodes on the path to the change (copy-on-write).

* **Snapshot** = `Document::clone()` = O(layers + symbols) pointer copies (0.4 µs measured on
  a 200k-instance document).
* **Change detection** = `PartialEq` that compares `Arc`s by pointer first (`arc_eq`), so it is
  O(changed), not O(document).
* **History** (`engine/src/history.rs`) stores snapshots and an *estimate of unique bytes* per
  step (memory not shared with the neighbouring state). It is bounded by a byte budget
  (default 512 MB) plus a step cap, evicting the oldest steps first.
* **State ids**: every committed change gets a process-unique `StateId`. Undo returns to the
  earlier id. The save point is a state id, so undoing back to the saved state makes the
  document clean again. Render caches key on state ids, so undo is a cache hit.
* **Gestures** coalesce a drag (or a slider scrub) into one undo step.
* **Atomic commands**: a failing command restores the pre-command snapshot. That is free with D.

## Consequences

* A rule every contributor must follow: never get `&mut` into the document except through
  `Arc::make_mut` (`Timeline::layer_mut`, `Layer::keyframe_mut`,
  `Keyframe::elements_mut_where`). Touch only what changes. For example, `Layer::normalize`
  only un-shares keyframes whose duration actually changes.
* The same sharing makes render-thread hand-off O(1)-ish (ADR 0002) and lets F6 keyframe
  copies share elements.
* Measured (release, 50 layers × 200 shapes × 240 frames = 200k instances): edit 0.011 ms,
  undo 0.009 ms, about 2.7 KB of history per nudge.
