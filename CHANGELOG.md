# Changelog

## Unreleased

* **Layer folders** (the folder button under the layer list, Insert › Timeline › New Folder,
  or a layer's right-click menu). Folders group layers, can nest, and open and close with
  their chevron. Hiding or locking a folder hides or locks everything in it. Move layers in
  and out with Up and Down: a layer moved in under a folder goes inside it, and moving off
  either end takes it out. A folder moves with its contents. Deleting a folder deletes what's
  in it (it asks first; undo brings it back). Masks work inside folders too. Animate
  documents keep their folders instead of flattening them. Up and Down are now icon buttons.

* **Scenes** (Insert › Scene; the scene menu at the top left of the stage): a movie is now a
  list of scenes that play in order, each with its own timeline. Switch scenes from the menu
  or with Shift+Page Up / Shift+Page Down. Add, duplicate, rename, reorder and delete scenes
  there or under Modify › Scene; deleting a scene with artwork asks first, and undo brings it
  back. PNG sequences and GIFs export the whole movie; single-frame exports use the scene
  you're in. Animate documents now import every scene, not just the first. Files are saved as
  format v3, and older files open as one scene.

* **Shape tweens** (Insert › Create Shape Tween, the timeline's Shape Tween button and
  right-click menus): one shape morphs into another between two keyframes, even with a
  different structure (a square into a star, one blob into two). Shape tweens show amber on the
  timeline; switch a span between classic and shape tween from the timeline's tween menu or
  Properties › Kind, and the easing is kept. Animate documents' shape tweens now import as
  shape tweens.

* **Import to Stage** (File › Import, **Ctrl+R** as in Animate; `file.import`): SVG drawings
  come in as editable shapes on a new layer named after the file, at the playhead, centred
  and selected, as one undo step. Supported: paths and basic shapes, groups, `use` and
  symbols, transforms, `viewBox`, inline and attribute styles, opacity, and linear and radial
  gradients. Anything not supported yet (text, images, filters, clip paths, dashes…) is listed
  in the status bar.

* **SVG export** (File › Export › Export Frame as SVG…, `file.export_svg`): any frame as a
  standalone vector SVG, matching the stage. Shapes, strokes, solid and gradient fills, opacity
  and tint, masks (as clip paths, nested inside symbols) and outline-mode layers all come
  across. Replacing an existing file asks first.

## 0.1.0 (2026-11-05): early alpha

The first version: milestones M0 (foundations) and M1 (authoring core) are complete. The
repository starts here with one commit; earlier work is credited in `AUTHORS.md`.

* **Drawing**: Selection, Subselection, Free Transform, Pen, Pencil (smooth, straighten, ink),
  Rectangle, Oval, PolyStar, Line, Paint Bucket (with gap closing), Ink Bottle, Hand and Zoom.
  Also cut, copy and paste (in centre, in place), duplicate, arrange, rotate and flip, align
  and distribute, convert to symbol and break apart.
* **Timeline**: layers (show, lock, outline), keyframes, blank keyframes, frame spans, classic
  tweens with easing presets, labels, onion skin, looping playback. Frames can be selected
  across layers; cut, copy and paste frames; drag keyframes and span ends. Timeline zoom and
  scrolling.
* **Mask layers**: a mask reveals only its fill area of the layers under it, on the stage and in
  every export.
* **Files**: `.anicraft` documents, PNG frames and sequences, animated GIF. Opens Animate
  documents (`.fla`, `.xfl`), with a report of anything not yet supported.
* **Ease of use**: an editable keyboard shortcut map (F1), with the familiar animation
  defaults. Also the command palette (Ctrl+K), right-click menus everywhere, confirmations only
  before real loss, an unsaved-changes guard, Preferences, and macOS, Windows and Linux
  conventions.
* **Engine**: every action is a command, scriptable headless through `anicraft-cli`. Undo is
  cheap even on large files. The stage renders in tiles in the background, on a deterministic
  CPU renderer.

Not in this version: scenes, layer folders, motion and shape tweens, text, bitmaps, sound,
SVG/Lottie/video export, plug-ins and installers (milestones M2 to M8 in `docs/roadmap.md`).
