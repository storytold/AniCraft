# Changelog

## Unreleased

* **CI on macOS and Windows** (ADR 0067, M8). Every pull request that changes code now builds,
  tests and lints the workspace on macOS (arm64) and Windows (x64) runners as well as Linux, so
  macOS-only and Windows-only code is checked before it merges. Documentation-only changes skip
  them, and a newer push cancels the older run. Their first run
  found one macOS-only failure: the screen-reader translation tests treated the macOS app
  menu's title, the product name, as untranslated.
* **The user guide** (ADR 0069, M8).
  * **Help › User Guide** opens AniCraft's guide in your browser. It comes with the app, so it
    works offline; the web editor serves it beside the editor. It is also in the command palette.
  * Eighteen chapters written for artists, from getting started, drawing, symbols, the timeline and
    tweens to masks, the camera and bones, text, images and sound, interactivity, exporting and
    Publish, opening Animate files, preferences and accessibility, plug-ins, working headless, and
    troubleshooting, plus **Coming from Animate**, which maps Animate's panels, tools and shortcuts
    to AniCraft's.
  * A **command reference** and a **shortcut reference** (with the macOS keys beside the others),
    generated from AniCraft itself, so they always match the version you have.
  * `anicraft-cli guide --out <folder>` writes the guide as HTML pages.
  * On macOS, the fixed keys that hold a key while dragging ("Space+Drag") keep that key in their
    label.
* **Brush library, pressure curves and the Paint Brush** (ADR 0068, M7).
  * **Window › Brush Library**: seven built-in brushes, each with a stroke preview drawn by the
    renderer: Pencil, Ink Pen, Flat Calligraphy, Round Taper, Marker, Chalk (a ragged edge that
    is the same every time) and Dotted. They are all made of parameters, with no bitmaps. A click
    sets the brush in hand.
  * **Your own brushes**: Save Brush keeps the brush in hand under a name, in your preferences.
    Right-click to rename or delete one (deleting asks first). Import… and Export… move a brush
    set between computers as a small JSON file.
  * **Pressure curves**: an editor in Properties maps pen pressure to the size and, optionally,
    to the stroke's opacity. It has Linear, Soft and Firm presets, or your own points.
  * **Shaping**: tapers at the start and end, dab spacing and a rough edge, in Properties.
  * **Velocity**: without a pen, drawing faster draws thinner (off by default).
  * **Paint Brush (Y)**: draws a line in the stroke colour whose width follows the pressure and
    tapers, as a width profile you can reshape with the Width tool.
  * Headless: `brush.preset_list`, `brush.preset_save`, `brush.preset_rename`,
    `brush.preset_delete`, `brush.preset_export`, `brush.preset_import`; `preset`, `curve`,
    `pressure_size`, `pressure_opacity`, `taper_start`, `taper_end`, `spacing`, `jitter` and
    `as_stroke` on `draw.brush`.
  * Pen tilt isn't read yet: egui doesn't report it.

* **Dynamic and input text fields** (ADR 0066, M5).
  * **Properties › Text type**: Static, Dynamic or Input text, as in Animate. Dynamic and input
    fields have an **instance name**, a **line type** (single line, multiline, multiline no wrap,
    password), a fixed **box**, **Show border**, **Selectable** and **Max characters**.
  * **Scripts** set and read fields: `score_txt.text = "Score: " + n`, `appendText`, `.length`.
  * **Test Movie**: click an input field and type. Caret, selection with Shift and the pointer,
    Backspace, Delete, the arrows, Home, End, Enter in multiline fields, Tab to the next field,
    Select All, Copy, paste and input methods. Screen readers find fields as text inputs.
  * **HTML canvas export**: input fields are real text inputs over the movie (input methods, the
    on-screen keyboard and the clipboard work), and text that scripts set is drawn in the
    browser. Authored text is drawn exactly as in AniCraft.
  * **Animate files**: dynamic and input fields come across as fields, with their names, boxes,
    borders and line types (they used to become static text).
  * Every other export and the stage draw fields with their authored text, box and border.
  * `text.field`, `text.create {type}` and `control.test_movie` `text` and `select_all` events
    for agents and the CLI.
* **Templates, SVG into the Library, and drops on the timeline** (ADR 0065, M8).
  * **File › New from Template** and the start screen's new **Templates** row: Leaderboard banner
    728 × 90, Rectangle banner 300 × 250, Social square 1080 × 1080 at 30 fps, Storyboard 16:9 (six
    labelled panels), Character turnaround and a Lip-sync mouth chart. Each has named layers and
    guides on a locked guide layer (they show while drawing and never export). They open as new,
    untitled documents with a drawing layer active.
  * **File › Save as Template** keeps the open document, with a picture, under a name and a
    category in your settings folder. It shows under "My templates" on the start screen, with
    Delete. Replacing or deleting a template asks first.
  * **SVG into the Library**: dropping an SVG on the Library, or File › Import › Import to Library,
    makes one graphic symbol named after the file, in the folder under the pointer.
  * **Drops on the timeline**: a sound dropped on a layer's frame goes onto that keyframe. An image
    or SVG goes onto that layer at that frame, in a new keyframe if needed. Either way it is one undo
    step, and a highlight shows the cell. Locked layers, folders and the camera say why they won't
    take it.
  * The timeline's layer footer keeps its Move Down button in every language.
  * Headless: `file.new_from_template`, `file.templates`, `file.save_as_template`,
    `file.delete_template`, and `file.drop` with `to: "timeline"`.
  * Exact drop positions from the desktop still wait for winit 0.31 (eframe 0.37 is still on 0.30).
* **Accessibility, second pass** (ADR 0064, M8).
  * **Select on the stage from the keyboard.** With the stage focused (Ctrl+F6), Tab goes to
    its elements, and a ring shows the focused one on the artwork.
    * Tab, Shift+Tab and the arrows move between elements.
    * Enter or Space selects one; Shift+Enter or Shift+Space adds it to the selection or
      takes it out.
    * Esc goes back to the stage.
  * **Screen readers hear what is on the stage.** While the stage has focus, its elements are
    a list ("Rectangle", "Text: Hello", "ball · Graphic: Ball"), each with its layer, place,
    size, on-screen bounds and whether it is selected.
  * **The active layer's frames are a list too.** Each item reads "Frames 1–24: keyframe ·
    motion tween · label “walk”", and activating one moves the playhead there.
  * **The timeline from the keyboard.** On the focused timeline, the arrows move the playhead
    and the active layer, and each move is announced.
  * **The menu bar from the keyboard** on Windows and Linux.
    * Press and let go of Alt, or press F10. Then the arrows, Enter and Esc work as in other
      applications.
    * Alt+F, Alt+E and the rest open a menu directly. While Alt is held, each title shows its
      letter underlined.
  * **Follow system** is the new default theme. AniCraft switches to high contrast while
    Windows uses a contrast theme, macOS has Increase Contrast on, the Linux desktop asks for
    high contrast, or the browser prefers more contrast. Choosing Graphite or High contrast
    keeps that palette.
  * Headless: `element.select` takes `toggle`.

* **Test Movie in its own window, and opening documents on macOS** (ADR 0062, M8).
  * **Control › Test Movie** (Ctrl+Enter) now opens in a **window of its own** on the desktop,
    as in Animate. It is sized to the stage at 100 % (smaller if that wouldn't fit the
    screen), titled with the document's name, and opens where it was last. It takes the
    keyboard when it opens. **Esc** or **Ctrl+W (⌘W)** closes it, Ctrl+Enter restarts it in
    place, and the editor's Ctrl and Alt shortcuts still work from it.
  * In the web build, and wherever the system can't open another window, it floats inside the
    editor as before and says why. **Preferences › Test Movie** keeps it inside the editor if
    you prefer. On Wayland the desktop places the window, so it can't come back where it was
    (Preferences says so).
  * **macOS:** double-clicking a `.anicraft`, `.fla` or `.xfl` in Finder, dropping it on the
    Dock icon, or choosing Open With › AniCraft opens it, whether AniCraft is running or
    not. If there are unsaved changes it asks first, and the Dock's Quit asks to save too.
    This uses the small `fmv-macos-events` crate (MIT OR Apache-2.0), so AniCraft's own code
    stays free of `unsafe`. Still to be checked on a Mac (the list is in ADR 0062).
  * Windows and Linux already open a double-clicked document from the command line; a test
    now covers it.

* **Timeline polish** (ADR 0061, M1).
  * A **per-layer onion skin toggle** in each layer row (Alt-click: this layer only, again:
    every layer). Layers left out show only their playhead frame; Edit Multiple Frames still
    edits them.
  * **View as Outlines** draws a layer's artwork as hairlines in its colour **on the stage
    only**. Exports, Test Movie and the player draw it as it is (before, an outlined layer
    exported as outlines). The **colour chip** on the row's edge opens the layer colour.
  * **Onion skin inside a symbol edited in place** now also shows the parent timeline's frames
    around it, dimmed.
  * **Layer Height** 100 / 200 / 300 % (right-click menu), **Layer Properties** (double-click a
    layer's icon, Properties… or Modify › Timeline), and a **Frame View** menu with frame widths
    Tiny to Large and **Short** rows. Frame zoom and Frame View sit at the right end of the
    timeline's bottom bar, as in Animate, so the header fits at the default window size again
    (the tween-kind menu was cut off).
  * Headless: `layer.set_onion`, `layer.set_color`, `layer.set_height`, `layer.set_properties`,
    `layer.properties`. Saved in the document with defaults left out, so older files open as
    before. XFL import reads a layer's height.
* **Animated PNG and animated WebP export** (ADR 0063, M4).
  * **File › Export › Export Animated PNG…** and **Export Animated WebP…** make full-colour
    animations with real transparency, for the web and chat apps, where a GIF has 256 colours
    and hard edges. WebP is lossless. Both are transparent by default; turn on the stage colour
    as background if you want it. Loop forever, play once, or play a set number of times.
  * Every frame is exactly what AniCraft renders. Frames that repeat the one before are stored
    once and shown longer, and each later frame stores only the rectangle that changed, so
    holds and small motions cost little.
  * Timing is exact: APNG delays are fractions of the frame rate (29.97 fps stays 29.97), and
    WebP's milliseconds never drift.
  * Both are in **Publish Settings** (Animated PNG, Animated WebP) with frames, scale, loop and
    background; in a browser they download. Headless: `file.export_apng`, `file.export_webp`
    and `anicraft-cli apng` / `webp`.
  * On the sample (48 frames, 1280 × 720) the APNG is 2.0 MB and the WebP 1.8 MB, against the
    GIF's 1.4 MB, which has only 256 colours; both take less time to make than the GIF.

* **Publish Settings and Publish** (ADR 0060, M8).
  * **File › Publish Settings** (Ctrl+Shift+F12) is a window that stays open while you work.
    It lists every format AniCraft exports (HTML canvas, animated GIF, video, Lottie, sprite
    sheet, PNG sequence, PNG, SVG, audio), each with a checkbox. The selected format shows
    its output name and the same options its File › Export command has. The settings are
    saved **in the document** and every change can be undone. Older files open with the HTML
    page on.
  * Output names are relative to the document's folder, and `{name}` stands for the
    document's name (`web/{name}.html`). Each name is checked as you type and the full path
    is shown below it. A name can't point outside the document's folder, and two formats
    can't write the same file.
  * **File › Publish** (Shift+F12) writes every format that is on, in the background. A
    window shows each format's progress and has a Cancel button. A single report then lists
    what each format wrote, skipped (video in a browser, audio with no sounds), left out
    (Lottie's and the HTML page's notes) or why it failed. One failing format doesn't stop
    the others.
  * Publishing over files that are already there asks once. After that, the files this
    session published are replaced without asking.
  * **Presets**: save the settings under a name, apply them to any document, or delete them.
    Presets are kept in your preferences.
  * In a browser, the files go to your downloads, as one ZIP when there are several.
  * Headless: `publish.settings_get`, `publish.settings_set` and `file.publish`, which returns
    the report. Publishing the sample in six formats takes 0.64 s.
* **Files from the desktop, recent files and a start screen** (ADR 0059, M8).
  * **Drop files onto the window**: `.anicraft`, `.fla` and `.xfl` open (asking first over
    unsaved work); SVG drawings, PNG and JPEG images and WAV, Ogg and MP3 sounds dropped on the
    **stage** go onto the active layer at the playhead, centred where they were dropped, and
    dropped on the **Library** go into the folder under the pointer. One undo step per drop.
    A highlight shows where they will land; anything AniCraft can't take gets one clear
    status line. In the web editor too.
  * **File › Open Recent**: the last 10 documents opened or saved, with pinned favourites on
    top, missing files greyed out with Remove, and Clear Recent Files. Kept in the
    preferences (browser storage on the web).
  * **Start screen** when AniCraft starts without a document: New with HD, Full HD, square
    and vertical presets or a custom size and frame rate, Open, the sample animation, and the
    recent files with a picture of their first frame (captured on save). Esc goes back to the
    document; File › Start Screen brings it back; it can be switched off.
  * New commands for the CLI and MCP: `file.drop`, `file.open_recent`, `file.recent_list`,
    `prefs.recent_pin`, `prefs.recent_remove`, `prefs.recent_clear`; `file.import` takes
    `cx`/`cy` and `layer: "active"`.

* **Buttons, complete: sounds on button frames and hit-area editing** (ADR 0057, M5).
  * A sound on a button's **Over** or **Down** frame plays when the pointer goes over or
    presses the button, in Test Movie and in the HTML canvas page (Web Audio). As the SWF format
    keeps them, the Up frame's plays on leaving and the Hit frame's on letting go over it. A
    stream on a button frame plays as an event. Buttons stay out of the soundtrack: Export Audio
    and video are unchanged.
  * Editing a button in place, the timeline names its four frames **Up, Over, Down, Hit** (and
    widens them to fit while the button is open), and the stage draws the **Hit area**
    translucent over the other states.
  * Test Movie has a **Show hit areas** checkbox.
  * XFL and `.fla` import bring sounds whose file is in the package (WAV, Ogg Vorbis, MP3), with
    the sounds on frames that use them, buttons' states included; sounds kept only in Animate's
    own format are reported.
  * Headless: `control.test_movie` lists the sounds it started.
  * 9-slice guides switched off are now kept, and come back where they were when switched on.
* **Find and Replace, and the Movie Explorer** (ADR 0058, M8).
  * **Edit › Find and Replace** (Ctrl+F) is a window that stays open while you work. It finds
    text in text fields, frame labels, layer names, instance names and frame scripts, with
    match case, whole word and regular expressions. It also finds fonts, colours (fills,
    gradient stops, strokes, text), symbols, sounds and bitmaps. It searches this scene and
    the symbols in it, or the whole document. Results show as you type.
  * Click a result, or press **Find Next** (F3) or Find Previous (Shift+F3), to go there: the
    scene, the frame and the item selected. A symbol opens in place through the instances
    that lead to it.
  * **Replace** replaces the current result and goes on to the next. **Replace All** is one
    undo step and asks first above 100 places. Replaced text is laid out again, colours keep
    their transparency, and a symbol is never put inside itself.
  * **Window › Movie Explorer** (Alt+F3) is a tab next to the Library. It shows scenes ›
    layers › keyframes › instances, text, sounds, bitmaps and scripts, each instance opening
    into its symbol, plus the Library's symbol definitions. A Show menu filters it by kind
    and a search keeps matching rows. Double-click a row to go there.
  * Both lists draw only the rows in view, so movies with 100,000 rows stay smooth.
  * Headless: `find.search`, `find.replace`, `find.goto` and `explorer.tree` (paged).
    Patterns use a linear-time engine with size limits.
* **Colour effects, complete** (ADR 0056, M2).
  * **Properties › Colour effect**: Animate's Style menu (None, Brightness, Tint, Advanced,
    Alpha) with each style's controls: brightness −100..100 %, tint colour and amount,
    Advanced's multiplier and offset for red, green, blue and alpha. Every row says "mixed"
    when the selection differs, and a change sets only that value on all of them. Switching
    style keeps the look where it can (Advanced starts as exactly the effect before).
  * One exact colour transform behind every style: nested effects compose exactly (a
    brightness around a tint shows both), and the CPU renderer, the GPU stage (bitmaps
    included, pixel-identical), SVG (`feColorMatrix` on bitmaps), HTML canvas (bitmaps
    recoloured, no longer reported) and Lottie (baked into the colours) draw the same.
  * Classic tweens interpolate within a style (a brightness tween passes through 0) and across
    styles in the Advanced space, as Animate does.
  * Motion tweens key **Brightness**, **Tint** colour (red, green, blue) beside its amount, and
    the eight **Advanced** values; the Motion Editor lists the styles in use and adds others
    with **Add Colour Effect**. A tween has one colour effect style, as in Animate.
  * XFL/FLA import brings every `Color` style across (Advanced no longer keeps only its
    alpha); a new `colour` corpus package with goldens.
  * `element.set_color_effect` for the CLI, MCP and plug-ins; `element.set_alpha` now keeps
    the rest of the effect. Older files open unchanged, and None, Alpha and Tint effects are
    still written exactly as before.

* **9-slice scaling for symbols** (ADR 0055, M2).
  * **Enable guides for 9-slice scaling** on a symbol, in the Library's item properties or in
    Properties. A scaled instance then keeps its corners unscaled, stretches its edges one way
    and its centre both ways. Rounded buttons and panels resize without distorting, strokes
    keep their width, and rotation and skew apply after. Nested symbols, text and bitmaps
    inside scale normally, as in Animate. When there isn't room for the corners, they shrink
    together in proportion.
  * While the symbol is open for editing, its four guides are **dashed lines across the
    stage**. Drag them with the Selection tool (one undo step per drag), or type their
    positions in Properties.
  * Every export draws it from the same frames. SVG and Lottie get baked outlines. HTML canvas
    export scales such instances as a whole for now and says so in its report.
  * XFL and `.fla` import read `scaleGridLeft/Right/Top/Bottom`. Headless:
    `symbol.set_scale_grid {symbol?, on?, left?, right?, top?, bottom?}`.
* **Frame scripts in ActionScript 3.0 run small logic** (ADR 0052, M6).
  * Typed variables (`var score:int = 0;` as Number, int, uint, Boolean, String or untyped),
    kept per timeline instance across its frame scripts as Animate timelines keep them;
    expressions (arithmetic, comparison, logic, joining text, `Math.min/max/abs/floor/ceil/
    round/random`, casts); `if`/`else`, `for`, `while` and `do` loops; functions with
    parameters and results, callable from any frame of the same timeline.
  * Scripts read properties of instances and timelines (`x`, `y`, `alpha`, `visible`,
    `rotation`, `scaleX/Y`, `currentFrame`, `totalFrames`, `currentLabel`, `mouseX/Y`, `name`),
    remove listeners, and listen for `Event.ENTER_FRAME` and for keys on the stage
    (`KeyboardEvent.KEY_DOWN`/`KEY_UP`, `e.keyCode`); `e.target` and `e.currentTarget` work.
  * Test Movie sends your keys to the movie (it says so in its bar); `Math.random()` is
    seeded, so a headless `control.test_movie` with the same `seed` and events always plays the
    same; key events and `seed` there too.
  * An endless loop or an error stops only its script, with a line in Output, and the movie
    plays on. A script that uses a name no frame of its timeline declares doesn't run, and the
    Actions panel shows the name in place.
  * The HTML canvas export runs the same scripts, keys included, checked against Test Movie
    step by step.

* **Repeating bitmap fills on the GL backend** (M3): the GPU stage wrapped a repeating
  bitmap with a signed remainder, which GLSL leaves undefined for negative values, so on
  OpenGL (llvmpipe in the FreeBSD VM) the texels at the bitmap's left and top edges came out
  wrong. It now wraps without one, and a test keeps every shader free of signed remainders.
* **Library and Properties polish** (ADR 0053, M2).
  * **Drag Library items onto the stage**: symbols and bitmaps land at the pointer on the
    active layer at the playhead, a sound goes onto that layer's keyframe; drag several at once.
    One undo step, and it works inside a symbol open for editing. The stage shows where they
    land and on which layer.
  * **Place on Stage** (the Library's button, right-click, or **Enter** in the list) puts the
    selected items at the centre of the view.
  * **Swap Bitmap** in Properties › Bitmap: a placed bitmap keeps its position and transform
    and takes the new bitmap's size; a shape painted with a bitmap fill keeps its outline.
  * **Convert to Symbol** (F8) asks for a name, a type and a **Library folder**, and offers the
    folder and type you used last.
  * **"mixed" on every Properties row** when several elements differ: Symbol, Type, Filters,
    Blending, Looping, First frame, Bitmap and Width now join Scale, Rotation, Alpha, Fill,
    Stroke and Style. A change sets them all; Type sets the type of every selected instance's
    symbol.
  * Headless: `library.place {items: [..], x, y}` (and `{sound}`), `element.swap_bitmap`,
    `kind` and `folder` on `modify.convert_to_symbol`; `symbol.set_kind` without `symbol` sets
    every selected instance's symbol.
* **`.fla` files open reliably, and hostile ones safely** (ADR 0054, M4).
  * The `.fla` reader takes the layouts zip tools write: folder entries, Windows `\`
    separators, CP437 and UTF-8 names (and Info-ZIP Unicode names), data descriptors and
    ZIP64. Each entry's size and checksum are checked.
  * Damaged or hostile archives fail with a sentence that names the problem: entries sharing
    bytes, sizes or checksums that lie, compression bombs (a 1024 : 1 ratio cap, 256 MB per
    entry, 1 GiB in all), paths with `..`, absolute or drive paths, duplicate names, more than
    32 nested folders, a central directory cut short. Nothing is ever extracted to disk.
  * A renamed `.fla` opens (it is recognised by its contents), and a `.fla` saved by Flash CS4
    or earlier says to open it in Animate and save it again.
  * Every corpus package, packed nine ways, imports identically to its folder and renders the
    same golden frames; a fuzz throws 9,664 hostile archives at the reader. A 54.6 MB `.fla`
    imports in 311–328 ms (checksums by slicing by 8: reading every entry 53 ms, was 130 ms).

* **Edit Multiple Frames, complete; onion skin inside symbols** (ADR 0051, M1).
  * With Edit Multiple Frames on, **Cut, Copy, Paste, Paste in Place and Clear** act on the
    artwork selected on every keyframe between the markers. Paste in Place puts artwork copied
    from several keyframes back keyframe by keyframe, and artwork copied on one frame onto every
    keyframe; Paste puts it on the current frame.
  * **Free Transform** shows one box around the selection on every keyframe and scales, rotates
    and moves it all about one pivot, one undo step per drag. **Align** (To Stage too),
    **Rotate 90°** and **Flip** work across keyframes, and so do the arrow keys.
  * Merge-drawing shapes moved, transformed or pasted there **merge again** on their own
    keyframe, so drawings stay connected.
  * **Onion skin and Edit Multiple Frames work inside a symbol** edited in place, on the
    symbol's own timeline.
  * Headless: `frames.transform`, `frames.align`, `frames.delete`, `frames.copy`, `frames.cut`,
    `frames.paste`, `frames.merge`.

* **Library management** (ADR 0050, M2).
  * **Folders**: New Folder (the folder icon under the list, or right-click), nested as deep as
    you like (32 levels). Drag items and folders onto a folder to move them in, onto an item to
    join its folder, or below the list for the top level; right-click › Move to does the same.
    Deleting a folder that holds items asks first, and its items go with it.
  * **Rename in place** with F2 or a double-click on the name; Enter keeps it, Esc cancels.
    Names are unique within a folder (case aside), as in Animate. Convert to Symbol picks the next
    free “Symbol n”.
  * **Duplicate Symbol** (right-click): “Ball copy” in the same folder, ready to rename.
  * **Swap Symbol** in Properties › Symbol: pick another symbol for the selected instances.
    Position, colour effect, blending, filters, looping and instance name stay.
  * A **use count** for every item, **Select Unused Items** and **Delete Unused Items…** (in
    More). Unused means nothing in any scene reaches it, so one pass clears a whole unused
    branch; symbols with a linkage class count as used. Deleting an item the movie uses asks,
    and its instances go with it.
  * **Sort** by name, type, use count or date added, ascending or descending, and **search**
    by name; folders that hold a match open.
  * A **preview** of the current item (a symbol's first frame, the bitmap, or a sound's
    waveform with a play button) and its **properties**: a symbol's type and linkage class, a
    bitmap's **Allow Smoothing**. Smoothing off keeps hard pixel edges on the stage, in
    renders, in SVG and in the HTML page.
  * Animate files keep their library folders and Allow Smoothing.
  * Headless: `library.items`, `library.new_folder`, `library.rename`, `library.move`,
    `library.duplicate`, `library.delete`, `library.unused`, `library.delete_unused`,
    `library.set_smoothing`, `element.swap_symbol`.
* **The GPU stage sorts heavy frames into tiles on the graphics card, and outlines are cheap**
  (ADR 0048, M3).
  * Frames with many shapes are binned on the GPU (count, prefix sum, scatter and tile-list
    compute passes) instead of on the CPU: the CPU only lays out the shapes' points, and every
    buffer is sized up front, so nothing is read back. On Vulkan, a 10,000-shape 4K frame
    drawn as one texture takes about 5 ms instead of 22, a 50,000-shape one about 23 ms
    instead of 122, and the 50,000-shape 4K stage about 37 ms instead of 75. Frames with few
    shapes, shapes inside masks or faded symbols, and Direct3D 12 (where the passes cost more
    per stage tile) are still binned on the CPU, which stays the reference
    (`ANICRAFT_GPU_BINNING=cpu|gpu|auto` to compare).
  * **Outline mode** (an outlined layer, and onion skin's Outlines) draws true one-pixel
    hairlines, one edge per segment where the old half-pixel strokes had four and a disc at
    every bend, in the CPU renderer and on the GPU alike. On a 2,000-shape scene with ±5 onion
    frames, outlines are now as fast as tinted onion frames: 10 ms on the GPU stage at 1080p
    (was 73) and 17 ms on the CPU stage (was 34–39). Lines look the same, with smoother joins.
  * Parity with the CPU renderer is unchanged (at most 1/255 on every test scene), and a test
    checks that the GPU's tile lists are the CPU's, entry by entry and pixel by pixel.
* **Raw points, snapping while drawing, and symbols edited in place** (ADR 0045, issue #42, M1).
  * **Add a point** on a merged drawing's line or fill edge with the Pen's Alt-click or a
    Subselection double-click: every shape on that edge gets it, and the curve keeps its shape.
  * **Delete points** with Delete (or the right-click menu): the shapes that share each point are
    joined across it, so a fill and its outline stay together; deleting a line's last points
    removes it. A point where three or more lines meet is kept, and the status bar says so.
  * **Make Smooth / Make Corner** on raw points. Dragging a handle of a smooth point keeps the
    other handle in line (on drawing objects too); **Alt** moves it alone.
  * The **Line, Rectangle, Oval, PolyStar and Pen** tools snap their points to points, lines and
    fill edges within 10 screen pixels, with a ring where they will snap. **Snap to Objects** is
    now Ctrl+Shift+/, as in Animate.
  * **Double-click a symbol instance** (or press Ctrl+E, or double-click the symbol in the
    Library) to edit it in place: the rest of the stage dims, the timeline shows the symbol's
    layers, and merge drawing, the Selection and Subselection tools and every other tool work
    inside it as on the stage. The edit bar shows `Scene 1 › Symbol`; click the scene, press
    Ctrl+E or double-click empty stage to go back. Undo returns to the symbol where a change was
    made. Rotated instances open upright.
  * Headless: `raw.add_point`, `raw.delete_points`, `raw.convert_points`, `draw.snap`, `smooth`
    on `raw.move_handle` and `path.move_points`, `symbol.edit`, `symbol.exit`; `doc.info` lists
    the open symbols.
* **Motion path points pinned to frames** (ADR 0049, M2). As in Animate, a motion tween's
  position keyframes are now points on its path: move the object at a frame and the path gets a
  point there that the object passes exactly on that frame, whatever else you do to the path.
  * Moving the object between the ends adds a pinned point where you put it; after the last
    position keyframe the path carries on to the new place. The eases on X and Y time the
    object between pinned points as they did between the ends.
  * Pinned points are diamonds with their frame number on the stage, and Selection can drag
    them. With Subselection, right-click any point to **Pin to Frame** (the playhead's frame)
    or **Unpin**. Properties › Frame has **Pin Here** / **Unpin**, and F6 inside a path tween
    pins where the object is.
  * The Motion Editor draws a pinned frame's X and Y keys in blue; dragging one moves its
    point. The timeline marks pinned frames with a larger diamond.
  * Adding and deleting points keeps the pins on their points; deleting a pinned point, or
    unpinning it, removes its frame's X and Y keys and the motion flows as before.
  * Older files open and play exactly as they did (only the ends are pinned).
  * Headless: new `motion.pin_point` and `motion.unpin_point`; `motion.info` and the path
    commands report `pins`.

* **Stroke caps, joins, miter limit, and dashed and dotted lines** (ADR 0046, M7). Properties ›
  Stroke has Animate's rows: **Style** (Solid, Dashed with a dash and gap editor, Dotted with
  dot spacing), **Cap** (None, Round, Square), **Join** (Miter, Round, Bevel) and **Miter**
  (the limit past which corners are bevelled). The drawing tools' options set them for new
  strokes, the Ink Bottle gives them and the Eyedropper picks them.
  * Drawn exactly by the CPU renderer and the GPU stage: butt and square caps, miter joins with
    their limit, dashes by arc length along curves (restarting on each subpath, an offset into
    the pattern), round dots one width across, and dashes that keep a width profile's widths.
    Patterns finer than a pixel draw solid and lighter. No slower than round strokes.
  * SVG, HTML canvas and Lottie export write caps, joins, miter limit and dashes; SVG import
    reads `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `stroke-dasharray` and
    `stroke-dashoffset`; Animate documents bring their caps, joins, miter limit, dashed and
    dotted strokes (ragged, stipple and hatched strokes are drawn solid and reported).
  * Merge drawing keeps a dashed line's style on every piece it is cut into; shape tweens blend
    miter limits and dash lengths. Older files open unchanged.
  * Headless: new `stroke.set_style`; `cap`, `join`, `miter`, `style`, `dash`, `dash_offset`
    and `dot_spacing` on `element.set_stroke`, the drawing commands and `paint.ink`.
* **Onion skin controls and Edit Multiple Frames** (ADR 0047, M1). Onion skin works as Animate
  users expect:
  * **Markers** on the frame ruler: drag the start and end brackets to choose how many frames
    show before and after the playhead. The onion menu (the arrow next to the timeline's onion
    buttons, a right-click on the ruler, or View › Onion Skin Options) has Always Show Markers,
    Anchor Markers (they stay put when the playhead moves), and the Onion 2, Onion 5 and Onion
    All presets.
  * Frames before the playhead are tinted blue and frames after it green, each fainter the
    farther it is. Pick your own colours or turn tinting or fading off in the menu; **Outlines**
    draws onion frames as outlines only, and **Keyframes Only** shows only keyframes. Your
    choices are remembered.
  * Onion frames now show over a background layer (they used to be hidden under it), and locked
    layers are left out, as in Animate.
  * **Edit Multiple Frames** (the timeline button next to Onion Skin) shows every keyframe
    between the markers at full strength. Click, Shift-click, marquee or Select All to select
    artwork on all of them, then drag it or use the arrow keys to move it everywhere at once:
    one undo step. A motion tween's path moves with its object.
  * Headless: `frames.artwork` lists the artwork on those keyframes and
    `frames.edit_multiple` moves it.
* **ActionScript that controls the timeline now runs** (ADR 0013 tier 2). Frame scripts in
  Animate documents, and ones you type in the Actions panel, play in Test Movie and in the
  HTML canvas export when they use the ActionScript 3.0 most animation and e-learning files
  use: `stop()`, `play()`, `gotoAndPlay`/`gotoAndStop` by frame or label (and scene),
  `nextFrame()`, `prevFrame()`, `nextScene()`, `prevScene()`, on this timeline or on named
  instances (`hero_mc.gotoAndStop("run")`, `MovieClip(parent).play()`); button and movie clip
  listeners (`btn.addEventListener(MouseEvent.CLICK, onClick)` for click, mouse down and up,
  roll over and out, with a named or inline function); `trace`; and setting `x`, `y`,
  `alpha`, `visible`, `rotation`, `scaleX` and `scaleY`.
  * A script runs whole or not at all. Anything else (variables, `if`, loops, reading
    properties) keeps it from running, and AniCraft says where: the Script view lists each
    problem with its line and column above the code and marks the lines (click one to go
    there), or says the script runs. Test Movie's Output begins with which scripts run and
    which don't, and why.
  * `navigateToURL` and other calls that reach outside the movie are ignored and noted.
  * The XFL import report lists the frame scripts that run apart from those only kept.
  * Headless: `script.set` and `script.list` say `runs` and `problems`; new `script.check`
    compiles a text without storing it. A new XFL corpus package, `interactive`, plays its
    ActionScript through the player.
* **Selection across layers** (ADR 0044, issue #42, M1). A selection can hold artwork on several
  layers, as in Animate:
  * The Selection tool's **marquee** selects on every visible, unlocked layer, cutting raw fills
    and lines at its edges on each layer (and taking the cut back when you let go of it).
    **Shift-click** adds or removes artwork on any layer; **Select All** (Ctrl+A) takes every
    unlocked layer. Locked and hidden layers are never selected; locking or hiding a layer lets
    go of what was selected on it.
  * Move, nudge, delete, duplicate, arrange (within each layer), transforms, alpha, fill and
    stroke, filters, blending, break apart and width profiles act on every selected layer, one
    undo step per gesture. Inside a tween each layer gets its own keyframe.
  * Align and distribute work across the layers; Free Transform's box covers the whole
    selection; copy keeps each layer's artwork apart, and paste puts it back on those layers
    (or on the active layer and the ones below, creating missing layers in another document).
  * **Convert to Symbol** (F8) of a selection on several layers keeps those layers inside the
    symbol and places the instance on the topmost of them.
  * The **timeline** shows a square in the layer's colour after the name of every layer with
    selected artwork. **Properties** says "6 elements on 4 layers", shows the combined box's
    position, and marks values that differ as "mixed".
  * Headless: `element.select` takes ids on any layer (and `layer?`); `doc.info` reports
    `selection_layers`; per-layer commands report the `layers` they ran on.
  * Clicking a layer's name in the timeline selects all of its artwork at the playhead and
    makes it active, as in Animate (`layer.select {select: true}`).
  * Not yet: "mixed" on every Properties row.

* **Interface languages** (ADR 0040, first slice; `docs/localisation.md`). Every string the
  window shows, and every name a screen reader announces, can now be translated.
  * Preferences › Appearance › **Language**: *System language* by default, or any language
    this build has, named in itself. A change shows at once. `--lang <code>` sets it for one
    run.
  * Translations come from the Craft family's shared **craft-i18n** catalogs, built in with
    `CRAFT_I18N_DIR`; a missing one shows the English. craft-i18n has no AniCraft catalog yet,
    so builds are English for now: `scripts/i18n-extract.sh` lists every string for
    translators (and, with a craft-i18n checkout, how much each language already covers).
  * A **pseudo-locale** (`--lang pseudo`) draws everything accented, longer and bracketed, to
    find text that skipped translation or gets cut off.
  * Messages that say how to undo show your Undo key as this computer writes it (⌘Z on a
    Mac, and a rebound key wherever it is), instead of a fixed "Ctrl+Z".
  * Menus, the command palette and the shortcut sheet translate the command registry's labels
    as they are drawn; command ids, `.anicraft` files, `anicraft-cli` and MCP stay English.
  * For developers: `tr!`, `trn!` (plurals by CLDR rules), `trc!` and `tr_noop!`; a test fails
    on new interface text that skips them. Not yet: engine messages, right-to-left languages
    (egui can't lay them out yet), Chinese, Japanese and Korean (need a font).

* **Bone tool and inverse kinematics, first slice** (ADR 0043, M7). Join symbol instances into
  an armature and pose it:
  * With the **Bone tool** (M), drag from one symbol instance to another: a bone joins them and
    both move to a new **armature layer** (bone icon in the timeline). Drag from a bone's end to
    the next instance to continue the chain; drag from any instance of the armature to branch.
  * Bones show on the stage as wedges from their round joint to their end, in the armature
    layer's colour; click one to select it (selection colour).
  * With the **Selection tool**, drag a bone or an instance to pose the armature by inverse
    kinematics: the bones above it turn so it follows the pointer; an unreachable point leaves
    the chain stretched straight toward it. Drag the root instance to move the whole armature.
    Each drag is one undo step.
  * A pose at another frame keys it; poses tween with the span's easing and are exact at their
    keyframes. Every export, the GPU stage and Test Movie show them.
  * **Properties › Bone**: the joint angle at the playhead, Constrain rotation with min and
    max (kept in every pose), Pin (the bone and those it hangs from stay put while you drag
    below it), and Remove Bone (also Delete). Removing frees the instances where they are, on
    a layer of their own; it asks first when the armature has more than one pose.
  * Headless: `bone.add`, `bone.remove`, `bone.set_limits`, `bone.pin`, `bone.pose`,
    `bone.solve` (IK) and `bone.info`; plug-ins may call them. A 64-bone chain solves in about
    5 ms from rest.
  * Not yet: shape armatures, spring, joint translation, runtime armatures, XFL import of
    armature layers, editing armature members' colour effects in place.

* **Subselection on raw drawings, Snap to Objects, and merging everywhere** (ADR 0042, issue
  #42). With Object Drawing off:
  * Click a raw line or fill with the **Subselection tool** (A) to see the points of everything
    connected to it: hollow squares, one per shared corner. Click a point to select it (Shift
    adds or removes). The curve handles of the selected points, and of the edge you clicked,
    show as lines ending in dots.
  * Drag a point to move it together with the other selected points; every shape that shares
    a point moves with it, and it snaps as with the Selection tool. Drag a handle to change the
    curve; a fill and its outline change together.
  * Drag a line or fill to move everything connected to it. A marquee shows the shapes it
    touches on the active layer and selects the points inside it. Arrow keys move the selected
    points 1 screen pixel (Shift: 10). Each drag or press is one undo step.
  * **Snap to Objects**, in View › Snapping, as the magnet under the tools and in the
    selection tools' options, turns snapping on and off. It is on by default.
  * **The Paint Bucket** fills a raw drawing exactly up to its lines and fill edges, and the
    fill joins any fill of its colour it touches. Recolouring a fill joins it to its new
    colour's neighbours.
  * **Pasted** raw shapes (Paste and Paste in Place) cut and join the drawing they land on.
  * **Animate files**: raw shapes open as raw shapes, merged into one drawing per keyframe.
    Drawing objects and groups stay objects.
  * **Gradient and bitmap fills** now merge like solid ones, each piece keeping its part of the
    gradient. Strokes with a width profile are still drawn as objects.

  Headless: `raw.subselect`, `raw.subselect_rect`, `raw.points`, `raw.move_handle`, `with` on
  `raw.move_point`, `draw.snap_to_objects`, and `object` on `paint.bucket`; plug-ins may call
  them.

* **The Selection tool on raw drawings** (ADR 0041, issue #42). With Object Drawing off, the
  Selection tool now edits raw strokes and fills the way Animate does:
  * Drag an unselected line or the edge of a fill to **bend** it. The segment becomes a curve
    through the point you drag, its ends stay put, and a fill under an outline (or next to it)
    bends with it. Straight lines stay straight until you drag them.
  * Drag an unselected **corner or end** to move it. Every raw shape that shares the point
    moves with it, so the drawing stays joined; where a line ends on an edge, the edge moves
    too.
  * Drag a fill to move only that fill, not its outline. Drag anything selected to move the
    selection.
  * A small corner or curve mark next to the pointer shows what a drag will do.
  * Dragged points and selections **snap** to corners, ends, lines and fill edges within 10
    screen pixels, at any zoom, with a ring where they snap. Arrow-key nudges join ends within
    0.2 px at their midpoint, and an end that comes within 0.2 px of a line goes onto it
    without changing the line.
  * The **marquee** selects the parts of raw fills and lines under it, cut at its edges, and
    objects wholly inside it; Shift adds more. If you click away without changing them, the
    drawing is whole again: selecting is never an undo step and leaves a saved file saved.
  * Each drag is one undo step, and what it changed merges again when you let go.

  Headless: `path.bend_segment`, `raw.move_point`, `raw.select_rect`, `raw.merge`, `raw.hit`,
  and `snap`/`from` on `element.translate`; plug-ins may call them.

* **Sound effects, layer volume and sounds inside symbols** (ADR 0039, M5 second slice).
  * Properties › Sound has an **Effect**, as in Animate: Left channel, Right channel, Fade left
    to right, Fade right to left, Fade in and Fade out, shaped over the time the sound is
    heard, or **Custom**. **Edit…** opens Edit Envelope: each channel's waveform with its volume
    line; click to add a point (up to 16), drag to move it, right-click to delete it, or type
    its time and levels. Play previews the sound with its envelope. Editing a preset makes it
    Custom. The timeline's waveform follows the envelope.
  * Layers with sounds have a **Volume** and **Mute** (Properties, and a speaker toggle in the
    timeline row). Muted layers are silent in playback and in every export.
  * **Sounds inside symbols play.** A graphic's play on its own frames as its parent shows
    them (loop mode and first frame apply; a stream reached mid-way is heard from there); a
    movie clip's on a loop from when it appears. Volumes multiply down through symbols.
  * **Test Movie has sound**: the sounds of the frames it enters, movie clips' and graphics'
    included, with the picture keeping time with the sound.
  * The HTML canvas page carries envelopes, volumes and sounds in graphics. Sounds on button
    frames are not played yet (reported). Sounds are not read from XFL files yet: Animate
    keeps their data in an undocumented form.
  * Commands: `sound.set {effect, envelope}`, `layer.set_volume`, `layer.toggle_mute`.
* **Scripts in Animate documents are kept** (ADR 0013 tier 1). Opening an `.fla` or XFL
  file used to drop its frame scripts. Now they come across as they were written, together
  with symbol linkage classes and the document class. Nothing runs them yet.
  * The scripts are frame scripts in scenes and in every symbol, in the language the
    document's publish settings name (ActionScript 3.0 unless they say otherwise). The
    import report lists them as "kept, not run". Scripts on instances, from older documents,
    are reported as left out.
  * Actions panel (F9): a keyframe's *Type* is Actions, Plug-in or **Script**. The Script view
    shows the code in a monospace editor with line numbers, shows its language, and edits it
    live. A burst of typing undoes in one step. Keyframes with a script get the same "a" on
    the timeline as keyframes with actions.
  * Properties shows a symbol's linkage *Class* and the document *Class* in documents that use
    scripts. Test Movie says how many kept scripts don't run, and the HTML canvas export
    lists them in its report.
  * Headless: `script.set`, `script.clear`, `script.list` and `script.set_class`. Plug-ins may
    call them too. Saved in `.anicraft` without a format change, so older files open as
    before.
  * A new XFL corpus package, `scripts`, has frame scripts in a scene and a movie clip, a
    linkage class and a document class, checked byte for byte and against golden renders.

* **Motion path points and moving the whole path** (ADR 0038, the second slice of motion
  paths). With the Subselection tool, double-click a motion path to add a point (the curve and
  the motion don't change), and Shift-click to select several anchors. Delete removes them: the
  path refits through the rest, and the ends stay, because they are where the motion starts and
  ends. Right-click makes points smooth or corners, or straightens the path. The Pen's
  Alt-click adds a point to a motion path or to a selected shape. Ctrl-drag (⌘-drag) the path
  or its object to move the whole path, and Properties › Path start moves it by number; the
  timing stays exactly. Headless: `motion.add_point`, `motion.delete_points`,
  `motion.convert_point`, `motion.move_path`, and `anchors` in `motion.info`. Not yet:
  anchors tied to frames (spatial keys; ADR 0038 explains why), a marquee over path anchors,
  nudging and transforming a whole path.

* **AniCraft in a web browser** (ADR 0036, first slice; `docs/web.md`). The editor builds for
  WebAssembly and runs in a page: `scripts/build-web.sh` (or `.ps1`) makes `web/`, which any
  static server can serve. The module is about 9 MB, 3.3 MB gzipped.
  * Open and import with the browser's file picker. Save and exports download the file; Save
    asks for the name once.
  * Autosave keeps recovery copies in the browser's storage. Work from a tab that closed or
    crashed is offered at the next start; a tab that is still open is left alone.
  * Preferences are kept in the browser too, and leaving the page with unsaved work asks first.
  * Not yet in the browser: sound playback, the GPU stage, video export and plug-in installs.
    The menu items that can't work there are dimmed and say why.
  * For developers: engine commands read and write files through `engine::vfs` (the disk on the
    desktop, memory in a browser). `file.open` takes `unsaved` and `untitled`. Every crate
    builds for `wasm32-unknown-unknown` in CI, and the engine's tests also run inside
    WebAssembly in Node.

* **Opening Animate files is checked against a corpus of test files** (XFL import, ADR 0035).
  Eleven hand-written Animate documents cover everything the importer brings across: shapes
  and strokes, primitives and groups, gradients, bitmaps, symbols in every loop mode, movie
  clips and buttons, colour effects, classic and shape tweens with eases and shape hints,
  masks (also in folders and symbols), guides and motion guides, text, filters, blend modes,
  scenes, labels and frame scripts. Each is imported, and chosen frames are rendered and
  compared with reference images, so a change that alters how an imported file looks is
  caught. Each file is also cut short and garbled thousands of ways to make sure opening a
  damaged file never crashes.

  Fixed while building it:
  * Symbols saved without a type now open as movie clips (Animate leaves the default out),
    not graphics.
  * Brightness on an instance now comes across (it was dropped).
  * Instance names of movie clips and buttons now come across, so actions can address them.
  * A frame comment no longer becomes a frame label.
  * A custom ease with one curve now comes across exactly.
  * The import report now also lists what is drawn differently from Animate: square and flat
    line ends and corners (drawn round), gradient strokes (one colour), repeating and
    reflecting, off-centre, stretched and linear-RGB gradients, advanced colour effects,
    reverse loop modes and extra spins on tweens.
* **Eraser and Eyedropper** (E and I, ADR 0034, first slice).
  * The Eraser sweeps the same tip as the Brush and takes what it covers out of the lines and
    fills on the active layer that were drawn with Object Drawing off. Drawing objects,
    symbols and text are kept, as in Animate.
  * Erase modes: Erase Normal, Erase Fills, Erase Lines, Erase Selected Fills and Erase Inside
    (only the fill where the stroke starts).
  * The Faucet option deletes a whole fill or line with one click.
  * Double-clicking the Eraser tool, or Erase Everything… in its options, clears the visible,
    unlocked layers at this frame. It asks first, and you can undo it.
  * The erased area shows as you drag, `[` and `]` change the size, and each stroke is one
    undo step. A stroke that erases nothing leaves no undo step.
  * The Eyedropper picks a fill's colour, gradient or bitmap into the fill swatch, or a line's
    colour, width and width profile into the stroke settings. It then becomes the Paint Bucket
    or the Ink Bottle.
  * A picked gradient stays where it is on the stage, so it carries on across the shapes you
    pour it into.
  * With shapes selected, the Eyedropper applies the picked style to them. Shift-click picks
    into the other swatch only.
  * For scripts and AI assistants: `draw.erase`, `draw.faucet`, `draw.erase_all`,
    `paint.pick` and `paint.apply_style`.
* **Brush tool** (B, ADR 0033, first slice). The brush paints filled shapes in the fill colour,
  as Animate's Brush tool does: the shape is exactly the area the tip sweeps along the drag.
  * Tip options: round or square, size, angle and roundness (for oval and flat calligraphic
    tips).
  * With a tablet, pen pressure sets the width (from 10 % to the full size); a mouse paints at
    full size.
  * Smoothing evens out a shaky hand.
  * Paint modes: Paint Normal covers lines and fills, Paint Fills leaves lines alone, and Paint
    Behind paints only empty areas.
  * With Object Drawing off, brush strokes merge with the drawing like the other tools'
    shapes; strokes of one colour join.
  * The paint shows as you drag, a click paints a dab, and `[` and `]` change the size. Each
    stroke is one undo step.
  * For scripts and AI assistants: `draw.brush`.

  Also fixed: a closed curve fit could add a straight edge across the shape when its first
  point wasn't a corner, which affected some Paint Bucket fills.
* **The stage draws on the graphics card by default** (GPU stage, third slice, ADR 0032).
  Bitmap fills and all seven filters (Drop Shadow, Blur, Glow, Bevel, Gradient Glow, Gradient
  Bevel, Adjust Color) are now drawn on the GPU too, so documents imported from Animate no
  longer fall back to the CPU. A 4K stage with filtered symbols and a bitmap backdrop draws in
  about 4 ms (the CPU needs 31 ms), and 200 bitmap fills in about 4 ms (37 ms). Bitmaps are
  uploaded to the graphics card once and kept. The picture is the same as the CPU's, within
  one 8-bit step (bitmaps exactly the same). Windows that run on Direct3D 12 now get the GPU
  stage as well. The GPU stage is in every build and on by default: Preferences › Stage ›
  Renderer offers *Graphics card (GPU)* (the default) and *CPU (reference)*. The CPU still
  draws whenever there is no usable graphics card (or only a software one), anything fails, or
  a tile needs more than the GPU handles (more than 16 different images, or filtered symbols
  nested more than 4 deep); the status bar says which. Exports always use the CPU.
* **Merge drawing** (ADR 0031, issue #42). The Rectangle, Oval, PolyStar, Line, Pen and Pencil
  tools now draw as Animate does:
  * shapes on a layer cut and join each other;
  * a fill drawn on top replaces what it covers, or joins a fill of the same colour;
  * lines cut each other where they cross, and a line right across a fill splits it in two;
  * a fill hides the lines under it;
  * points within 0.2 px become one.

  Click a fill or a stroke segment to select just that; selected shapes show a dot pattern
  instead of a box. Double-click a stroke for every stroke joined to it, or a fill for the fill
  and its outline. Drag or nudge a shape and it cuts into what it lands on. Each step is one
  undo.

  **Object Drawing** (J, or the tool's options) switches back to shapes that keep to themselves.
  Arrow keys now move the selection one screen pixel (Shift: ten) at any zoom.

  Headless: the drawing commands take `object` (scripts and AI assistants still draw objects
  unless they pass `"object": false`), plus `draw.object_drawing`, `draw.drop` and
  `element.select_connected`.

  Not yet: bending segments by dragging them, snapping to points and strokes, moving shared
  points together, partial marquee selection, and merging Paint Bucket fills, pastes and raw
  shapes from Animate files.

## 0.1.0 (2026-11-09)

* **Motion paths for motion tweens** (ADR 0030). Select a motion tween's object (or a frame of
  its span) and the stage shows its path in the motion colour, with a dot for every frame.
  Drag the path with the Selection tool to bend it and its ends to move where the motion
  starts and ends; with the Subselection tool drag its points and handles (smooth points stay
  smooth; Alt breaks them). Each drag is one undo step. The object travels the path at the
  pace its X and Y curves set, so eases (overshoot included) carry over, and a straight path
  moves exactly as before. Moving the object at a later frame reshapes the path instead of
  adding keys. **Orient to path** (Properties) turns the object as the path turns; Straighten
  goes back to the straight path. Every export and Test Movie play it. Headless:
  `motion.set_path {d}`, `motion.clear_path`, `motion.orient {orient}`; `motion.info` reports
  the path and where the object is. Not yet: adding or deleting path points on the stage,
  anchors tied to frames, motion paths from Animate files.
* **GPU stage, second slice** (ADR 0029; builds with `--features gpu-stage`, Preferences ›
  Stage › Renderer › GPU (preview)). The graphics card now draws each stage tile straight into
  the picture the window shows, with no copy back to the CPU: a 4K stage with 1,000 shapes
  takes about 2 ms, and 10,000 shapes about 10 ms (the CPU needs 16–28 ms). **Mask layers**
  (nested, several shapes per mask), **symbols that fade as one** and **all 14 blend modes**
  are drawn on the GPU too, so ordinary documents no longer fall back to the CPU; masked scenes
  draw up to 7× faster than on the CPU. The picture is the CPU's within one 8-bit step.
  Filters and bitmaps still draw on the CPU (the status bar says so). When the window runs on
  OpenGL, the GPU stage uses a device of its own and copies tiles back, as before; Preferences
  says which.

* **HTML canvas export** (ADR 0028). File › Export › Export HTML Canvas… writes the movie as
  one web page that plays in any modern browser: open it from your disk, put it on a server or
  send it as one file. Everything moves exactly as on the stage, because every frame of every
  timeline is computed by AniCraft (classic, shape and motion tweens, easing, motion guides,
  shape hints); shapes, gradients, strokes (variable-width ones too), text, bitmaps, masks,
  symbols that fade or blend as one, alpha and tint all come across. It is interactive: movie
  clips play on their own timelines, buttons show Up, Over and Down and answer in their Hit
  area, and frame actions and button events (stop, play, go to a frame or label in any scene,
  next/previous frame and scene, trace, set properties, instance paths like `panel.door`)
  behave as in Test Movie: the page's player is a port of AniCraft's and the tests run both
  side by side. Sounds play with the first click (Event, Start, Stop; Stream over its span).
  The movie fits the browser window, sharp on high-density screens. The demo exports in 1 ms
  to a 59 KB page (18 KB of movie data). Filters, Subtract and Invert blending, tints on
  bitmaps and plug-in frame scripts are reported, not exported. Headless:
  `file.export_html {path, title?, sound?, fit?}`.

* **Camera layer with layer depth** (ADR 0027). Click the camera button in the timeline
  header (or pick the **Camera tool**, C) to add a camera: a Camera layer on top of the
  timeline whose frame shows on the stage as an orange outline of what the movie will show.
  With the Camera tool, drag to pan it, turn it with the round handle above the frame and zoom
  with the wheel (each drag is one undo step); Properties › Camera sets position, zoom (5–2,000
  %), rotation and a tint over the whole view, or resets it. The camera animates like any
  object: add keyframes on its layer and create a classic or motion tween (the zoom is Scale X
  in the Motion Editor). Give layers a **Depth** (Properties › Frame, once there is a camera):
  a layer behind the stage moves less as the camera pans, for parallax. **View › Preview
  Through Camera** shows the stage as the movie will look. Every export (PNG, sequences, GIF,
  video, sprite sheets, SVG, Lottie), Test Movie and the GPU stage see the camera; the stage
  stays in stage space while you edit. Hiding the camera layer turns the camera off; removing
  an animated camera asks first. Headless: `camera.add`, `camera.remove`, `camera.set`,
  `camera.info`, `layer.set_depth`.
* **Installers for Windows, macOS and Linux** (ADR 0026, `docs/release.md`). A push to the
  `release` branch now builds installable AniCraft for every platform and drafts a GitHub
  Release: a Windows **MSI** (x64 and arm64) that installs for your account without an admin
  prompt, or for everyone if you choose, with a Start menu entry; a macOS **DMG** with the
  universal app (drag to Applications); a Linux **AppImage**, **deb**, **rpm**, **Flatpak** and
  tarball. Everywhere, `.anicraft` documents get AniCraft's icon and open in AniCraft, and FLA
  and XFL files list AniCraft under "Open with" without changing your default app.
  `anicraft-cli` is installed next to the app. Windows and macOS builds are signed (macOS also
  notarized) and the Linux packages and checksums are signed when the release secrets are set.
  `anicraft --version` and `anicraft-cli --version` print the version. On Windows, AniCraft no
  longer opens a console window beside the app. On Linux the window now
  reports the app id `ai.storyteller.anicraft`, so docks show AniCraft's icon. Not yet: on macOS,
  double-clicking a document starts AniCraft but doesn't open the file.

* **Bevel, Gradient Glow and Gradient Bevel filters** (ADR 0025). Properties › Filters ›
  Add filter now offers all seven of Animate's filters on movie clip and button instances.
  **Bevel** lights the edges that face the light and shades the others (shadow and highlight
  colours with opacity, angle, distance, blur, strength, quality); **Gradient Glow** colours a
  glow by a ramp, from where it fades out to where it is strongest; **Gradient Bevel** colours
  a bevel by a ramp, highlight at the left, shadow at the right. Each can sit inside the
  artwork's edge, outside it or both (Type), and Knockout shows only the effect. The gradient
  filters open with a ramp editor: click the strip to add a colour stop, drag a marker to move
  it, pick one to change its colour and opacity or remove it; the glow's first stop and the
  bevel's middle stop stay transparent, as in Animate, so nothing is painted away from the
  edges. SVG export writes them as standard SVG filters, Animate documents keep them, and
  `.anicraft` files save them. Headless: `element.set_filters` with `bevel`, `gradient_glow`
  and `gradient_bevel` (colours as `#rrggbbaa`).
* **Fixed**: a filter reaching very far (a long drop shadow distance, or several large
  filters) could stop the whole frame from drawing; it is now cut far off the stage instead.
  Inner glows and inner shadows came out weaker near the edge of their drawing area; blurs
  of an even width were half a pixel off centre; and exported SVG cut shadows that reached
  far from small artwork.
* **Interactivity: actions, buttons and Test Movie** (ADR 0024). Make a movie stop on a
  frame, wait for a click and go on, without writing code. **Window › Actions** (F9) edits
  what a keyframe does when the playhead reaches it and what a button or movie clip does on
  Click, Press, Release, Pointer over and Pointer out: add Stop, Play, Go to and play, Go to
  and stop, Next/Previous frame or scene, Trace or Set property from a menu, and pick the
  target instance, frame, label or scene from your document. Name instances under
  **Properties › Instance** to steer movie clips by name (`hero.arm`); keyframes with actions
  show an "a" on the timeline. **Control › Test Movie** (Ctrl+Enter) plays the movie in its
  own window at its frame rate, with buttons showing their Up, Over and Down frames and
  answering clicks within their Hit area, and an **Output** panel for traces and anything
  that couldn't run; Restart, Pause and Step are at the top, and your stage stays as it was.
  **Frame scripts**: a WebAssembly plug-in of kind `frame-script` can run on a keyframe,
  read where the movie is and send actions back, within 20 million instructions per frame
  (`docs/plugins.md`). Headless: `actions.set`, `actions.clear`, `actions.list`,
  `instance.set_name`, `control.test_movie` (plays with timed clicks and returns the output),
  and `file.new_sample {"which": "interactive"}`. Exports still show the timeline as
  authored.

* **Shape hints** (Modify › Shape; ADR 0023). On a shape tween, Add Shape Hint (Ctrl+Shift+H)
  adds a lettered marker, `a` to `z`, at the shape's centre. Drag it onto an edge on the
  tween's first keyframe and onto the matching edge on its last: the morph then carries that
  point there, so a square becomes a circle without turning and letters morph without folding.
  Markers are rose until placed, amber once placed on the first keyframe and mint on the last;
  they snap to the nearest edge as you drag, and right-click removes one or all (Ctrl+Z
  undoes). Headless: `tween.add_hint`, `tween.move_hint`, `tween.remove_hint`,
  `tween.clear_hints` and `tween.hints`. Hints are saved in `.anicraft` files (older files open
  unchanged) and come across from XFL `shapeHints`.

* **Accessibility, first pass** (ADR 0022, `docs/accessibility.md`). **High contrast** theme in
  Preferences › Appearance › Theme: white text on black, yellow focus, selection and active
  tool, outlined buttons and fields, brighter timeline blocks; every text colour at 7:1 or more.
  **Screen readers** (Narrator, VoiceOver, Orca through AccessKit) now hear a name and role for
  every control: tools ("Pen tool (P)"), layer rows and their Show/Lock/Outline toggles,
  transport and icon buttons, every Properties field ("Position X", "Fill colour"), Library rows,
  the Motion Editor, every dialog; the stage reads as a canvas with the tool, frame and
  selection, and status messages are announced. **Ctrl+F6 / Ctrl+Shift+F6** (⌘F6 on macOS)
  move the keyboard between Stage, Timeline, Properties, Library and Tools, listed and
  rebindable in the shortcut sheet; shortcuts keep working while a button has focus. **Interface
  scale** holds up to 200 %: the stage refits, the tools column and timeline header scroll, the
  status bar shortens its message instead of overlapping. The default theme now meets WCAG AA
  everywhere (the playhead's frame number, the current-frame readout and faint hints were
  below it). A test walks the AccessKit tree in 23 states of the interface and fails on any
  unnamed control. `anicraft --theme high-contrast --scale 200` previews a look for one run.
  Not yet: stage elements and single frames one by one for screen readers, following the
  system's contrast setting, the menu bar from the keyboard.
* **Variable-width strokes and the Width tool** (**U**, ADR 0018). A stroke's width can now
  vary along it: pick a profile in Properties › Width (Uniform, Taper both ends, Taper start,
  Taper end, Bulge, Pinch, each with a picture), or set one in the drawing tools' options so
  new lines start tapered. With the Width tool, hover a stroke to see its width points; drag a
  point's handle to widen or narrow the line there (Alt: one side only), drag its dot to slide
  it along, or drag anywhere on the line to add a point. Delete removes the selected point, and
  each drag is one undo step. Changing the stroke width scales the whole profile. Shape tweens
  blend profiles, SVG and Lottie export write the strokes as filled outlines matching the
  stage, and `.anicraft` files keep them. Headless: `stroke.set_profile`,
  `stroke.set_width_point`, `stroke.delete_width_point`, `stroke.width_at`, and a `profile`
  param on `element.set_stroke`, the drawing commands and `paint.ink`.
* **Fixed**: perfectly horizontal or vertical lines inside a moved shape could not be picked
  (their flat bounds were not transformed).
* **GPU stage, first slice** (M3, ADR 0019; build with `--features gpu-stage`). Preferences ›
  Stage › Renderer (also View › Stage Rendering) can draw the stage on the graphics card
  instead of the CPU. It draws fills, strokes and gradients from the CPU renderer's own
  preparation and matches it to within one 8-bit step at a few edge pixels; masks, symbol
  opacity, blend modes, filters and bitmaps still draw on the CPU, and the status bar says
  which drew the stage and why. A 4K frame of 1,000 shapes takes about 9 ms instead of 18 ms.
  Exports always use the CPU renderer. `ANICRAFT_STAGE_RENDERER=gpu|cpu` overrides the choice.
* **Clip paths, masks and text in SVG drawings** (ADR 0021). Import to Stage (Ctrl+R) now
  brings clipped artwork across on mask layers: a drawing with clip paths comes in as a folder
  named after the file, with each clipped group on a masked layer under a mask layer holding
  the clip's shape, so you can move or reshape the mask afterwards. Clips that cut nothing
  (the frame clip design tools add) are dropped, and clips inside clips are combined exactly
  when they can be. Masks come across when they are solid shapes (white for luminance masks);
  soft or faded masks are drawn unmasked and reported. Text comes in as text you can edit,
  one block per positioned line, with its font, size, weight, italic, colour, letter spacing
  and anchor (start, middle, end); fonts that aren't on this computer fall back to the default
  and the status bar names them. A frame exported as SVG with masks imports back as the same
  picture. Per-character positions, text on a path and dashed strokes are reported, not yet
  imported.

* **Sound** (ADR 0020). Import WAV, Ogg Vorbis or MP3 sounds: **File › Import › Import to
  Stage** puts one on a new layer at the playhead, its span as long as the sound; **Import to
  Library** just adds it. A sound belongs to a keyframe, as in Animate: pick it under
  **Properties › Sound**, with **Sync** (Event, Start, Stop, Stream) and **Repeat ×N** or
  **Loop**. The timeline draws each sound's **waveform** over its frames. Playback plays the
  sounds in sync (the playhead follows the sound), stream sounds pick up wherever the playhead
  jumps, and dragging the playhead plays short snippets of them. The Library lists sounds with
  Preview, Place on timeline and Delete. **File › Export › Export Audio** writes the
  soundtrack as a WAV, and **Export Video** now includes it (Opus in WebM, AAC in MP4, by your
  ffmpeg). Sounds are saved inside the `.anicraft` file as imported. Headless:
  `file.import_to_library`, `sound.place`, `sound.set`, `sound.remove`, `sound.rename`,
  `sound.delete`, `file.export_audio`. Linux builds need the ALSA development package (or
  `--no-default-features` for silent playback).
* **Plug-ins** (Commands › Plug-ins; ADR 0017, `docs/plugins.md`). Plug-ins are WebAssembly
  files that add commands: generators such as the sample **Star Field**, and selection effects
  such as the sample **Jitter Selection**. A plug-in with settings opens a dialog with a live
  preview on the stage; OK keeps the result as one undo step named after the plug-in. Install
  with Commands › Install Plug-in…, see and remove them in Commands › Manage Plug-ins…; they
  load from the `plugins` folder in AniCraft's settings folder (or `ANICRAFT_PLUGINS`), and the
  command palette finds them too. Plug-ins run sandboxed: they change the open document through
  AniCraft's own commands, never files, the clipboard or the undo history, and a runaway or
  broken plug-in stops with a message and leaves the document as it was. Headless:
  `plugin.install/list/run/remove/reload`, `anicraft-cli --plugins <folder>`. The samples and a
  small Rust SDK are in `examples/plugins`.
* **Motion tweens and the Motion Editor** (ADR 0016). Insert › Create Motion Tween (also in
  the timeline header, the right-click menu and Properties) makes a span animate its object
  property by property: X, Y, scale, rotation, skew, alpha and tint each have their own curve
  with property keyframes anywhere in the span. Move, scale, rotate or fade the object at a
  later frame and it keys those properties there (no timeline keyframes); F6 inside the span
  keys every property. The Motion Editor (a tab beside the Timeline, Window › Motion Editor, or
  double-click a motion span) lists the properties with their values at the playhead (drag one
  to key it) and draws the selected property's curve: drag keys up and down, drag a segment's
  two Bézier handles to shape its ease (overshoot allowed), pick an ease preset, click to move
  the playhead, double-click to add a key. Each drag is one undo step. Motion spans are sky blue
  on the timeline with a diamond at each property keyframe. Several objects become one graphic
  symbol first, and a one-frame span grows to a second, as in Animate. Every export sees the
  motion. Headless: `timeline.create_motion_tween`, `motion.set_key`, `motion.remove_key`,
  `motion.set_ease`, `motion.info`. Not yet: motion paths on the stage, moving keys in time,
  importing motion tweens from Animate files.
* **Crash recovery.** AniCraft keeps a recovery copy of unsaved work in the background (every
  minute, or after 25 changes, whichever comes first; Preferences › Files › Autosave changes
  both or turns it off). If AniCraft closes unexpectedly, the next start shows what was left
  behind: **Restore** opens it as unsaved work, and Save puts it back where it was (or asks for
  a name if that file was saved after the copy); **Discard** deletes it after a second click;
  **Later** keeps it for File › Recover Autosaved Documents…. Copies live in the settings
  folder, are written atomically, never replace your own files, and go away when you save or
  close normally. Drawing never waits for them: the 65 MB stress document is copied in 0.18 s
  on a background thread. Headless: `file.recovery_list`, `file.recover`,
  `file.discard_recovery`, `file.autosave`.
* **App icon**: an engraved rabbit on AniCraft yellow, on the same rounded tile as the other
  Crafting Apps. It shows on the window, the macOS Dock (on Apple's icon grid), the Windows
  taskbar and `anicraft.exe`, and in Help › About. Files and how to regenerate them are in
  `assets/app-icon/`.
* **README**: screenshots of the app and the sample animation, and a quick start.
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
