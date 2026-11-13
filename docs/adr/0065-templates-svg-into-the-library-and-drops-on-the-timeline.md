# ADR 0065: Templates, SVG into the Library, and drops on the timeline

* Status: accepted (2026-11-11)
* Relates to: ADR 0004 (destructive actions), ADR 0021 (SVG clips and masks), ADR 0040
  (interface languages), ADR 0050 and 0053 (Library folders, Place on Stage), ADR 0059 (files
  from the desktop, recent files, start screen), roadmap M8

## Context

ADR 0059 left four things next: templates on the start screen, SVG into the Library, drops on the
timeline, and exact desktop drop positions. Artists coming from Animate start banners, social posts
and storyboards from templates, keep their own studio setups as templates, drag an SVG into the
Library to get a symbol, and drag a sound onto a frame of the timeline.

## Decision

1. **Built-in templates are code, not files** (`anicraft_doc::templates`, beside the sample
   documents). Six, all original: Leaderboard banner 728 × 90 and Rectangle banner 300 × 250 (24 fps,
   15 s, the safe area a tenth of the short edge in), Social square 1080 × 1080 at 30 fps (safe
   margin, thirds), Storyboard 16:9 (six labelled 2-second panels on a Panels layer, action-safe
   93 % and title-safe 90 %), Character turnaround (five views on lines a head apart, eight heads
   tall), and Lip-sync mouth chart (a Mouth graphic symbol with ten labelled keyframes holding
   placeholder shapes, placed at Single Frame on a face guide). Guides sit on a locked guide layer,
   so they show while drawing and never export. Nothing binary is committed, every template is
   tested to build, open, draw on, save and reopen, and they cost under 0.1 ms to build. When a
   template opens, the first unlocked drawing layer is active, so drawing works at once.
2. **User templates are plain `.anicraft` files with a picture**:
   `<Templates>/<category>/<name>.anicraft` and `<name>.png` in the settings folder (or
   `ANICRAFT_TEMPLATES_DIR`). We chose folders over a manifest because they need no new format,
   survive a copy to another computer, and can be tidied in a file browser. Category and name must
   be valid file names everywhere: at most 64 characters, none of `/ \ : * ? " < > |`, no control
   characters, no leading or trailing dot, no Windows device names. They are refused with a reason,
   never silently changed. As with recent files, the engine never looks for the folder: the shell
   passes it to `file.save_as_template`, `file.templates` and `file.delete_template`.
   `file.new_from_template` takes a built-in `template` id or any `.anicraft` `path`, and opens it
   untitled (Save asks for a name and never writes over the template). It isn't a recent file, and
   it asks first over unsaved work, like New. Replacing a template and deleting one are irreversible
   and always confirm (ADR 0004). The document itself doesn't change when it is saved as a template.
3. **A template's picture shows its guides.** `cmds_template::picture_png` renders the first frame
   with the CPU oracle at the recent files' 240 × 135, with guide layers drawn as artwork and their
   lines widened by 1.5 ÷ scale, so a 2-pixel guide on a 1920-wide stage still reads. That takes
   0.4 to 1.1 ms per template. Built-in pictures are rendered once per run; user pictures are
   written on save.
4. **File menu and start screen.** New from Template sits under New from Sample and Save as Template
   under Save As, in registry order, as in Animate. New from Template opens the start screen
   scrolled to its Templates row: the built-in tiles, then "My templates" with Delete. One screen
   picks everything, and Esc goes back. Save as Template is a small dialog with the picture, the
   name, the category (the categories used before offered as chips), where it will be saved, and a
   note when it will replace one. The built-in names and descriptions are English in the engine,
   listed with `tr_noop!` in `src/templates.rs` for the catalogs, and translated where drawn. The
   default category is offered in the interface language because it becomes a folder name. On the
   web there is no folder: the built-in templates work, and Save as Template says why it isn't
   there.
5. **SVG into the Library is one graphic symbol.** `file.import_to_library` and drops on the Library
   take `.svg`. The result is a graphic symbol named after the file (a free name, numbered),
   holding the drawing's shapes with the registration point at its centre (as Convert to Symbol
   puts it), and its clip and mask layers inside it (ADR 0021). A drop puts it into the folder under
   the pointer.
6. **Drops on the timeline** (`file.drop` with `to: "timeline"`, `layer`, `frame`; one undo step)
   move the playhead and the active layer to the drop. A **sound** goes onto the keyframe the frame
   is in, as Animate puts it, and a keyframe is made only past the layer's end. One sound per drop,
   since a keyframe holds one. **Images and SVGs** go into a keyframe **starting** at that frame,
   made there when the frame is inside another keyframe's span. It is made as Insert Keyframe makes
   it, keeping what the frame showed, so the art before and after stays. The new artwork is centred
   in the view. A drawing that needs several layers or masks can't sit on one layer, so it goes in
   as a graphic symbol instance (and into the Library). We don't add layers the user didn't drop
   on. Folders, the camera, armature layers and locked layers refuse, saying why, and nothing
   changes. The shell records each timeline row and the frame mapping as it draws
   (`drop::TimelineGeo`). The highlight outlines the row and fills the frame's cell, with "Place … on
   Star at frame 9" or "Put whoosh.wav on Star's keyframe at frame 9", or the refusal in red. Over
   the layer column a drop means that layer at the playhead.
7. **Exact desktop drop positions stay out.** winit 0.30.13 is the version eframe 0.36 uses, and
   eframe 0.37 too. It reports `HoveredFile`/`DroppedFile` with a path and no position, and Windows
   and macOS send no pointer moves while dragging. ADR 0059's wait for the first pointer move after
   the drop stays. When eframe moves to winit 0.31 (which carries drag positions), `drop::frame`
   can read them, and the timeline highlight will follow the pointer on the desktop as it already
   does in the browser.
8. **The layer footer fits every language.** Its icon buttons (New folder, Up, Down) are laid out
   first from the right edge of the layer column, and "+ Layer" and Delete share the rest,
   shortening with an ellipsis if they have to. Before, in the pseudo-locale the longer text pushed
   the Down chevron off the column. Each keeps its full accessible name and tooltip.

## Consequences

* Templates, SVG symbols and timeline drops work headless (CLI, MCP) like every command. The new
  commands are in the `file` namespace, which plug-ins may not call.
* `file.*` commands are left out of `panic_hunt_every_command` because they touch the disk. The
  template and timeline-drop commands have their own hostile-params test, writing only to a
  temporary folder. Screenshot and test runs never touch the real templates folder.
* A template is only as portable as the files it refers to. Bitmaps and sounds are inside the
  `.anicraft`, so it carries everything.

## Not yet

* Template descriptions for user templates, renaming or moving a template between categories, and
  a "Show in folder" button.
* Exact desktop drop positions (winit 0.31), dropping folders of images as a sequence, and a sound
  dropped across several layers.
* Previews that play a template's animation, and more built-in templates (presentation, mobile
  banners, HTML5 canvas ads with click tags).
