# Changelog

## Unreleased

* **Text in Animate documents.** Opening a `.fla` or XFL now brings its text across as text
  you can edit: titles, captions and paragraphs keep their font, size, colour, bold and
  italic, alignment, letter and line spacing, position, and fixed width (wrapping) or
  auto-size. A field whose lines use different styles (a big title over smaller lines)
  becomes one text block per style, stacked as in Animate; a line that mixes styles takes
  the style of most of its characters, and the report counts it. Fonts that aren't on this
  computer fall back to the default, and the status bar names them ("Fonts not on this
  computer, drawn in Arial: Futura."). Dynamic and input text come across as static text and
  are reported. The import report now also shows in the status bar when a document is
  opened from the command line (ADR 0014).

* **ActionScript on the roadmap** (ADR 0013, low priority, M6): first keep the scripts in
  imported Animate files, with an Actions panel, then play the common timeline controls (stop,
  play, go to a frame or label, button clicks) in Test Movie and HTML canvas export. Until
  then, opening an Animate document now says how many frame scripts it left out instead of
  dropping them silently.
* **Filters** (Properties › Filters, on movie clip and button instances as in Animate): **Drop
  Shadow**, **Blur**, **Glow** and **Adjust Color**, in any order and combination, with blur
  X/Y, quality (Low, Medium, High), strength, colour, angle and distance, and inner, knockout
  and hide object. They show on the stage and in every picture export; SVG export writes
  matching SVG filters (blur, outer shadows and glows, colour adjustments), and Lottie export
  reports them as not exported. A masked instance is filtered first, then masked, as in
  Animate. Animate documents keep their instances' filters (bevel and gradient filters are
  reported). Headless: `element.set_filters`.

* **Blend modes** (Properties › Blending, on movie clip and button instances as in Animate):
  Normal, Layer, Darken, Multiply, Lighten, Screen, Overlay, Hard Light, Add, Subtract,
  Difference, Invert, Alpha and Erase. The instance is composited on its own and then blended
  with what is below, on the stage and in every export. SVG export writes the CSS blend modes
  it has, and Lottie export sets each shape's blend mode. Animate documents keep their
  instances' blend modes. Headless: `element.set_blend`.

* **Text** (the Text tool, **T**). Click on the stage and type; Esc or a click elsewhere
  finishes, and Enter starts a new line. Click text with the Text tool, or double-click it with
  the Selection tool, to edit it. Properties sets the font (any font on your computer), size,
  colour, bold, italic, alignment, letter and line spacing, and a width that wraps lines; with
  the Text tool in hand they set what new text gets, and new text keeps the last style used.
  Text moves, scales, rotates, fades, tweens and masks like any artwork, and every export
  shows it. Documents keep the letter shapes, so text looks the same on computers without its
  font. Break Apart (Ctrl+B) turns text into a shape. Headless: `text.create`, `text.edit`,
  `text.fonts`.

* **Export Lottie Animation** (File › Export, `file.export_lottie`): the movie, or a range of
  its frames, as a Lottie animation (JSON) for websites and apps. Every frame matches AniCraft:
  shapes, strokes, solid and gradient fills, colour effects, opacity and masks come across,
  each piece of artwork as its own layer that changes exactly where it changes in the movie.
  Transparent by default, or with the stage colour behind. Anything that can't come across
  exactly (images, masks inside masks, symbols fading as one, stretched round gradients) is
  named in the status bar.

* **AniCraft for AI assistants** (`anicraft-cli mcp`): a Model Context Protocol server, so an
  assistant can open a document, draw, animate, look at frames and export, through the same
  commands as the app, with undo and the same "are you sure" rules. See `docs/mcp.md`.

* **Images in Animate documents.** Opening a `.fla` or XFL now brings its bitmaps into the
  Library, with bitmap instances on the stage and bitmap fills on shapes (placed, scaled and
  tiled as in the file). Images are read when the document keeps them as PNG or JPEG (XFL
  folders keep the original files); bitmaps kept only in Animate's own internal format are
  listed in the report after opening. Bitmap fills now carry their own placement inside a
  shape (`matrix` on `Paint::Bitmap`); older files are unchanged.

* **Images in the Library.** Imported images are listed in the Library with their size, next
  to symbols, and the newest is picked. Double-click one, or press Place on stage, to add
  another copy centred on the stage; copies share the one image (`library.place` takes
  `bitmap` as well as `symbol`).

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
