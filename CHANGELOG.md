# Changelog

* **Moho import** (ADR 0101). `File › Open` now reads Moho documents: `.moho` packages and

## Unreleased

* **Layer animation and the camera** (ADR 0102). Moho documents with animated layers now
  play: each layer's translation, scale, rotation, shear and flips import as property
  channels (the Moho model, one curve per property — the same shape bones and switches will
  use), around the layer's own origin pivot, with Moho's interpolation modes mapped (linear
  and Bézier exactly; auto-smooth as a Catmull-Rom approximation; step as holds; Moho's ease
  family and exotic modes approximated, all counted in the import report). The scene camera
  imports too — position, zoom (Moho's 2 = neutral 100 %) and roll — moving every layer it
  looks at, except layers marked camera-immune. Timeline markers and the project's frame
  range come across, so a 44-frame Moho document plays 44 frames. Headless: `camera.set_track`,
  `camera.set_zoom`, `camera.set_roll`, `camera.clear`, `layer.set_transform`,
  `layer.clear_transform`, `scene.set_frames`, `scene.clear_frames`, `scene.add_marker`,
  `scene.remove_marker`. `.anicraft` files are format v5 (older files open unchanged). What
  is still approximated: Moho's exact auto-smooth formula and its ease/noisy/cycle modes —
  the import report counts them and the document keeps the original channels in its sidecar.

* **Moho import** (ADR 0024). `File › Open` now reads Moho documents: `.moho` packages and
  bare `.mohoproj` files, alongside `.fla`/`.xfl` and AniCraft's own format. The stage, the
  inline layer tree (flattened, folders kept), vector meshes as paths with solid and linear/
  radial gradient fills, solid strokes, text layers (laid out with this computer's fonts) and
  image layers all come across; what the document model can't hold yet — bones, keyframed
  channels, layer effect stacks, reflected/angle gradients, physics, camera animation — is
  counted in the import report instead of guessed at, and the untouched original JSON rides
  in the file as the `maop_maohou` sidecar (`.anicraft` format v4; older files open
  unchanged) so later milestones can re-project it.

## Unreleased

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
