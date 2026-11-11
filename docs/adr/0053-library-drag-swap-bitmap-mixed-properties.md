# ADR 0053: Library drag and drop, Swap Bitmap, Convert to Symbol folders, mixed Properties

* Status: accepted (2026-11-11)
* Relates to: ADR 0050 (Library management), ADR 0044 (selection across layers), ADR 0045
  (symbols edited in place), ADR 0020/0039 (sounds on keyframes), ADR 0004 (destructive actions),
  ADR 0022 (accessibility), ADR 0040 (interface languages), roadmap M2

## Context

ADR 0050 left the everyday Library gestures Animate users reach for first: dragging an item
onto the stage, swapping the bitmap an instance shows, choosing a folder when converting to a
symbol. ADR 0044 showed "mixed" on only some Properties rows (Scale, Rotation, Alpha, Fill,
Stroke, Style), so a selection of movie clips with different blending looked alike. Behaviour
was learned from Adobe's public help pages and from watching the program; nothing was copied.

## Decision

1. **One command places what a drop carries.** `library.place` takes one item
   (`{symbol | bitmap | sound: id, x?, y?}`, as before) or a list
   (`{items: [{symbol | bitmap | sound: id}, ..], x?, y?}`, up to 1,000). Symbols stand with
   their registration point on `x`/`y`, bitmaps are centred on it, all in the active layer's
   keyframe at the playhead (auto-key rules as for any edit), and everything placed is selected.
   A sound goes onto that keyframe (a blank keyframe is made on an empty frame, the span grows
   to the sound's length, as Place on Timeline did); a keyframe holds one sound, so two
   different sounds in one drop are refused. Positions are in the edited space, so inside a
   symbol open for editing (ADR 0045) a drop lands where the pointer is. Folders can't be
   placed; locked layers, folders, the camera and armature layers refuse as for drawing. One
   command means a drop of many items is one undo step, and the CLI, MCP and plug-ins (the
   `library` namespace) can do it too.

   *Rejected:* a gesture of one `library.place` per item. It would be one undo step in the
   shell only, and a script dropping several items would get several.

2. **Dragging onto the stage.** Library rows already carry their selection as a drag payload
   (folder moves). The stage accepts the same payload: while it is over the stage the pointer
   shows a copy cursor, a cross where artwork lands and "Place on <layer>" (for sounds alone,
   "Put on <layer> at frame n"); the rows' names follow the pointer everywhere. Letting go runs
   `library.place` at the pointer, mapped into the edited space. Folders in the payload are
   left out. While previewing through the camera, artwork edits are paused (ADR 0027) and the
   drop says so instead.

3. **The keyboard way (ADR 0022).** Place on Stage (the Library's button, its right-click menu,
   and **Enter** while the list has the keyboard) places the selected rows at the **centre of
   the view** (what the stage shows, in the edited space), no longer the stage's centre: what
   you see is where it lands, as a drop at the centre would put it. With several rows selected
   it places them all, one undo step.

4. **Swap Bitmap** (`element.swap_bitmap {bitmap}`, per layer, ADR 0044's "where it applies"
   class). A bitmap placed from the Library is a rectangle of the bitmap's size filled with it
   (identity fill matrix, no repeat). Such a shape keeps its transform, so its top-left corner
   (Animate's registration point for bitmaps) stays put, takes the new bitmap's size, and its
   name follows when it was the old bitmap's. Any other bitmap-filled shape (painted with a
   bitmap fill) keeps its outline and fill transform, and only the bitmap changes. Properties
   shows a **Bitmap** row under Fill with the bitmap's name and size and **Swap…**, listing the
   Library's bitmaps by path, as Swap Symbol does.

5. **Convert to Symbol (F8) is a dialog** with Name, Type (Graphic, Movie clip, Button) and
   Folder (the Library's folders by path, or the top level). It offers the folder and type used
   last, while that folder still exists (view state, `convert_symbol::Last`, not saved).
   `modify.convert_to_symbol` takes `kind` and `folder`; a name the folder already holds gets a
   number ("Ball 2", `Document::unique_name`) rather than an error, and the dialog says so
   before you press OK. Without a name it is the next free "Symbol n" in that folder. Without
   `folder` it stays at the top level, so scripts behave as before.

6. **"Mixed" on every Properties row** that edits several elements (`src/mixed.rs`, pure and
   tested per row): Scale, Rotation, Alpha (the colour effect; tint has no row yet), Symbol
   (Swap…), Type, Filters, Blending, Looping, First frame, Fill, Bitmap, Stroke (colour and
   width), Width (profile) and Style. Each row compares only the elements it edits: Blending and
   Filters movie clips and buttons, Looping and First frame graphics, Fill, Stroke, Width and
   Style shapes, the Bitmap row bitmap fills. So a shape beside a movie clip never makes
   Blending mixed. Fill compares the whole paint (on beside off, two bitmaps, two gradients).
   Position never says mixed: with several elements it is the combined box (ADR 0044).

   Editing a mixed row sets all, as the hint says. Choosing the value the field already shows
   applies it too when the row is mixed (otherwise a menu pick of the first element's value did
   nothing). For **Type**, `symbol.set_kind` without `symbol` now changes the symbols of
   **every** selected instance in one step (it was the first one's, ADR 0044 decision 2). The
   type belongs to the symbol, as in our model since ADR 0013, so this changes those symbols'
   other instances too, as picking the type in the Library does. Filters edits the first
   element's list and gives it to all.

## Consequences

* Dragging from the Library to the stage, Place on Stage with Enter, Swap Bitmap, a folder in
  Convert to Symbol and mixed rows behave as an Animate user expects, headless included.
* Tests: `crates/engine/src/place_tests.rs` (a drop of symbols, a bitmap and a sound in one
  undo step; positions; refusals that change nothing; locked layers; inside an open symbol;
  Swap Bitmap on a placed bitmap and on a tiled fill on two layers; Convert to Symbol into a
  folder, with clashing names, types and the file round trip), `src/mixed.rs` (one test per
  row group, including "editing a mixed row sets all"), `src/convert_symbol.rs` (the
  remembered folder). `panic_hunt_every_command` fuzzes mixed item lists, two sounds, far
  points, 1,500 items, and Swap Bitmap on a placed bitmap.
* `library.place`'s result gains `elements` and `sound`; `element` stays. `symbol.set_kind`'s
  result gains `symbols`.

## Not yet

Dropping files from the desktop onto the stage, a ghost picture of the item while dragging
(the name follows the pointer), snapping a drop to objects, a New Folder button in the Convert
to Symbol dialog and a registration-point grid, Swap Bitmap from the Library's own menu
(Animate's "Swap Bitmap" on the item, which swaps every use), "mixed" on text style rows and on
the instance name and events, and a Colour Effect row with tint and brightness.
