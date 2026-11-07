# Changelog

## Unreleased

* **Images.** Import to Stage (Ctrl+R) now takes **PNG** and **JPEG** too. An image arrives on
  its own layer, centred and selected, and can be moved, scaled, rotated, faded, tweened and
  masked like any artwork. It renders smoothly at every size, on the stage and in every export
  (SVG embeds it). Images are saved inside the document as PNG, with JPEGs converted losslessly.
  The model also has **bitmap fills** (`Paint::Bitmap`, tiled or once), so any shape can show an
  image.

* **Export Video** (File › Export, `file.export_video`): the movie as **WebM** (VP9,
  royalty-free; the default) or **MP4** (H.264), made by your own installed ffmpeg. AniCraft
  bundles no video encoder. ffmpeg is found through the `ffmpeg` parameter, the
  `ANICRAFT_FFMPEG` setting, or the system path, and if it's missing you're told how to get it.
  Frames stream in order straight from the renderer; odd sizes are padded to even.

* **Movie clips and buttons behave like themselves.** A movie clip instance plays its own
  timeline on a loop from when it appears, and keeps playing across keyframes that hold the
  same instance. A graphic keeps step with the timeline it's on. A button shows its Up frame.
  Properties now sets a selected instance's symbol **Type**, and a graphic's **Looping**
  (loop, play once, single frame) and **First frame** (`symbol.set_kind`, `element.set_loop`).

* **Symbols fade as one.** A symbol instance below full opacity now composites its contents
  first and then fades the result, as in Animate, so overlapping parts no longer show through
  each other. This applies on the stage and in PNG, GIF, sprite sheet and SVG exports (SVG
  uses `<g opacity>`). Fully opaque instances render exactly as before.

* **Export Sprite Sheet** (File › Export, `file.export_sprite_sheet`): movie frames in a grid
  on one PNG, plus a JSON frame map beside it in the common "JSON hash" layout, with frame
  rectangles, sizes and per-frame durations, ready for game engines. Choose the frame range,
  scale, columns, padding and whether to keep the stage background (transparent by default).
  Replacing either file asks first.

* **Motion guides** (a layer's right-click menu › Add Classic Motion Guide, or Insert ›
  Timeline). Draw a line on the guide layer and the layer's classic tweens travel along it,
  from the point nearest each start keyframe to the point nearest its end. With **Orient to
  path** (Properties) the artwork turns as the path bends. The guide shows on the stage while
  you animate and never appears in exports. Plain guides, and the Guide and Guided types, are
  in the right-click menu. Animate documents keep their guided layers and orient-to-path.
* The Pen path command (`draw.path`) now reads full SVG path data, including relative
  commands, H/V, S/T and arcs.

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
