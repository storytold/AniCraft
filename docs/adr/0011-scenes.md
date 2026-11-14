# ADR 0011: Scenes

* Status: accepted (2026-11-06)
* Relates to: roadmap M2 (scenes) and M4 (XFL scenes), ADR 0001 (persistent document), ADR 0003
  (file format)

## Context

Animators split longer work into scenes: parts of one movie that play in order, each with its
own main timeline (titles, shots, credits). Animate documents often have several, and until
now the importer kept only the first.

## Decision

1. **Model.** `Document.scenes: Vec<Arc<Scene>>`, where a `Scene` is a name and a `Timeline`.
   There is always at least one scene (`sanitize`). Symbols stay document-wide, shared by all
   scenes. Layer and element ids stay unique across the whole document, so duplicating a scene
   gives the copy fresh ids (mask links are remapped).
2. **Which scene is open is editing state, not document state.** `Session.scene` sits beside
   the playhead and the active layer. Switching scenes is not an undo step and does not mark
   the document dirty. Undo snapshots record the scene, so undoing an edit returns to the
   scene it was made in.
3. **Two notions of time, named apart.** `evaluate(doc, frame)` and `Document::frame_count()`
   are the *movie*: every scene in order, which is what exports play. `evaluate_scene` and
   `Session::frame_count()` are the scene being edited: the stage, the timeline and the
   playhead. Render jobs carry movie frames (`scene_start + frame`), so cached frames of
   different scenes never collide. Onion skin stays within the scene.
4. **Exports.** PNG sequences and GIFs cover the movie; single-frame PNG and SVG exports take a
   frame of the current scene.
5. **Files.** Format v3 stores `scenes`. Version 1 and 2 files (one `timeline`) load as
   "Scene 1", so nothing written before scenes is lost.

## Consequences

* Every editing command targets the open scene through `doc.timeline(s.scene)` and
  `doc.timeline_mut(s.scene)`, the only two ways in.
* Playing in the editor plays the open scene; *Test* is still per scene. A
  whole-movie test player can come later without changing the model.
