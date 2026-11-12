# Coming from Animate

If you already know Adobe Animate (or Flash before it), most of AniCraft will feel familiar. This
chapter maps what you know to where it is in AniCraft, lists the keys that work the same, and is
honest about what differs and what isn't there yet.

AniCraft is an independent, open-source project, and is not affiliated with or endorsed by Adobe.

On macOS, press ⌘ where this guide says Ctrl, and ⌥ where it says Alt.

## Panels

AniCraft keeps its panels docked in one fixed layout: tools on the left, the stage in the middle,
Properties and the Library on the right, and the timeline at the bottom. You don't open or close
most of them; they are always there.

| In Animate | In AniCraft |
|---|---|
| Tools | The tools column on the left. A drawing tool's options show at the top of Properties while it is active. |
| Timeline | The timeline at the bottom (**Window › Timeline**). |
| Properties | Properties on the right, always shown. |
| Library | The Library on the right (**Window › Library**). |
| Motion Editor | A tab beside the timeline (**Window › Motion Editor**). |
| Actions (**F9**) | The Actions panel, same key (**Window › Actions**). |
| Movie Explorer (**Alt+F3**) | Same key; a tab next to the Library (**Window › Movie Explorer**). |
| Output | The Output panel inside Test Movie. |
| Align | The Align section on the right, and **Modify › Align**. |
| History | The History section on the right. |
| Color and Swatches | The Fill and Stroke rows in Properties, and in each drawing tool's options. |
| Transform and Info | Position, scale and rotation rows in Properties, and the Free Transform tool. |
| Scene | The scene menu at the top left of the stage, **Insert › Scene** and **Modify › Scene**. |
| Find and Replace | **Edit › Find and Replace** (**Ctrl+F**). |
| Publish Settings | **File › Publish Settings…** (**Ctrl+Shift+F12**). |

Not in AniCraft yet: saved workspaces and rearranging panels, Components, Code Snippets and the
Assets panel. See [The workspace](workspace.md) for a tour.

## Tools and their keys

Each tool has the key you already know.

| Tool | Key | Notes |
|---|---|---|
| Selection | **V** | Selects raw strokes and fills with the dot pattern; drag an edge to bend it. |
| Subselection | **A** | Points and handles, on raw drawings and drawing objects. |
| Free Transform | **Q** | Scale, rotate and move; Shift and Alt as you'd expect. |
| Width | **U** | Variable-width strokes, with profiles in Properties. |
| Bone | **M** | Joins symbol instances into an armature. |
| Pen | **P** | Alt-click adds a point to a path. |
| Text | **T** | Static, dynamic and input text. |
| Pencil | **Shift+Y** | Smooth, Straighten and Ink modes. |
| Brush | **B** | Like Animate's Classic Brush: paints filled shapes, with pen pressure. |
| Eraser | **E** | Five erase modes and the Faucet. |
| Rectangle | **R** | With corner radius. |
| Oval | **O** | |
| PolyStar | (none) | Polygons and stars. |
| Line | **N** | Shift snaps to 45°. |
| Paint Bucket | **K** | With gap closing. |
| Ink Bottle | **S** | |
| Eyedropper | **I** | Picks fills, gradients, bitmaps and line styles. |
| Camera | **C** | Adds a camera layer. |
| Hand | **H** | Hold **Space** with any tool, too. |
| Zoom | **Z** | |
| Object Drawing | **J** | Toggles between merge drawing and drawing objects. |

Not in AniCraft yet: the Lasso, Polygon and Magic Wand tools, Gradient Transform, the 3D tools,
Asset Warp, the Paint Brush and Fluid Brush, Deco and the Spray Brush. More on the tools in
[Drawing](drawing.md).

## Keys that work as in Animate

| Action | Key | Menu in AniCraft |
|---|---|---|
| Insert frame | **F5** | Insert › Timeline › Frame |
| Remove frame | **Shift+F5** | Insert › Timeline › Remove Frame |
| Insert keyframe | **F6** | Insert › Timeline › Keyframe |
| Clear keyframe | **Shift+F6** | Insert › Timeline › Clear Keyframe |
| Insert blank keyframe | **F7** | Insert › Timeline › Blank Keyframe |
| Convert to symbol | **F8** | Modify › Convert to Symbol… |
| Actions panel | **F9** | Window › Actions |
| Play and stop | **Enter** | Control › Play |
| Previous and next frame | **,** and **.** | Control |
| Test Movie | **Ctrl+Enter** | Control › Test Movie |
| Publish | **Shift+F12** | File › Publish |
| Publish Settings | **Ctrl+Shift+F12** | File › Publish Settings… |
| Import to Stage | **Ctrl+R** | File › Import › Import to Stage… |
| Edit symbols | **Ctrl+E** | Edit › Edit Symbols |
| Break apart | **Ctrl+B** | Modify › Break Apart |
| Duplicate | **Ctrl+D** | Edit › Duplicate |
| Paste in place | **Ctrl+Shift+V** | Edit › Paste in Place |
| Deselect all | **Ctrl+Shift+A** | Edit › Deselect All |
| Redo | **Ctrl+Y** | Edit › Redo |
| Cut, copy and paste frames | **Ctrl+Alt+X**, **Ctrl+Alt+C**, **Ctrl+Alt+V** | Edit › Timeline |
| Align left to bottom | **Ctrl+Alt+1** to **Ctrl+Alt+6** | Modify › Align |
| Rotate 90° CW and CCW | **Ctrl+Shift+9**, **Ctrl+Shift+7** | Modify › Transform |
| Remove transform | **Ctrl+Shift+Z** | Modify › Transform › Remove Transform |
| Bring forward, send backward | **Ctrl+Up**, **Ctrl+Down** | Modify › Arrange |
| Bring to front, send to back | **Ctrl+Shift+Up**, **Ctrl+Shift+Down** | Modify › Arrange |
| Add shape hint | **Ctrl+Shift+H** | Modify › Shape › Add Shape Hint |
| Find and Replace, Find Next | **Ctrl+F**, **F3** | Edit › Find and Replace |
| Movie Explorer | **Alt+F3** | Window › Movie Explorer |
| Snap to objects | **Ctrl+Shift+/** | View › Snapping › Snap to Objects |
| Zoom in and out | **Ctrl+=**, **Ctrl+-** | View |
| Preferences (Windows and Linux) | **Ctrl+U** | Edit › Preferences… |

AniCraft also has **Home** and **End** (and **Shift+,** and **Shift+.**) for the first and last
frame, and **Shift+PageUp** and **Shift+PageDown** for the previous and next scene.

### Keys that differ

* **Ctrl+K** opens AniCraft's command palette, which finds any command by name. Align is always on
  the right, and on **Ctrl+Alt+1** to **6**.
* **Ctrl+G** and **Ctrl+Shift+G**: AniCraft has no groups yet. Use a graphic symbol (**F8**), or
  draw with Object Drawing (**J**) to keep shapes apart.
* **Ctrl+F8** (New Symbol): not there yet. Draw something and convert it with **F8**.
* **Ctrl+J** (Document Settings): it is **Modify › Document Settings**, with no key by default.
* Panel toggles such as **Ctrl+L** for the Library aren't needed, because the panels are docked.
* **Ctrl+0** fits the stage in the window and **Ctrl+1** shows it at 100 %.

Any key can be changed in **Help › Keyboard Shortcuts** (**F1**), so you can bring back a habit
of your own. See [Preferences, shortcuts, accessibility and languages](preferences-shortcuts-accessibility.md)
and the full list in [the shortcut reference](reference/shortcuts.md).

## Concepts that work differently

### Raw drawings and drawing objects

As in Animate, the Rectangle, Oval, PolyStar, Line, Pen, Pencil and Brush tools draw **raw**
strokes and fills by default: they cut and join the shapes they touch on the same layer, a fill on
top replaces what it covers, and lines split where they cross. The Selection tool shows a dot
pattern on selected parts, a double-click selects connected strokes (or a fill with its outline),
and dragging an edge bends it. Points snap to points, lines and fill edges within 10 screen pixels.

Press **J** for Object Drawing to draw shapes that stay separate. Scripts and the command line
draw objects unless told otherwise.

This behaviour was rebuilt to match Animate closely, and the remaining differences are tracked in
issue 42 on [GitHub issues](https://github.com/storytold/AniCraft/issues). One known gap:
variable-width strokes don't take part in merging yet.

### Classic, motion and shape tweens

All three are there, in **Insert**: **Create Classic Tween**, **Create Motion Tween** and **Create
Shape Tween**.

* **Classic tweens** (violet spans) work between two keyframes, with easing presets and custom
  curves, and follow a classic motion guide.
* **Motion tweens** (sky-blue spans) animate one object property by property, with property
  keyframes, the Motion Editor and a motion path you can bend on the stage. Moving the object at a
  frame pins a point of the path to that frame, as in Animate.
* **Shape tweens** (amber spans) morph between shapes, with shape hints.

The one big difference: motion tweens in Animate documents don't come across yet, so a `.fla` that
uses them opens without that motion, and the import report says so. See [Tweens](tweens.md).

### Symbols

Graphic, movie clip and button symbols behave as you expect: graphics keep step with their parent
timeline, movie clips play their own, and buttons have Up, Over, Down and Hit frames. Double-click
an instance (or press **Ctrl+E**) to edit it in place. Not yet: shared libraries and importing
Library items from another document. See [Symbols and the Library](symbols-and-library.md).

### ActionScript

AniCraft runs a subset of ActionScript 3.0 in frame scripts: timeline control (`stop()`,
`gotoAndPlay` and friends, on named instances too), mouse and keyboard listeners, `trace`,
setting and reading instance properties, variables, `if`, loops and functions. Scripts in an
Animate document are always kept with it. One that uses more than the subset doesn't run at all
(never halfway), and the Actions panel shows each problem with its line.

The Actions panel also offers a simple list of actions, so you can stop a movie or make a button
go to a frame without writing code. Not yet: arrays, objects, `switch`, string methods,
ActionScript 1.0 and 2.0, and scripts placed on instances. See [Interactivity](interactivity.md).

### Files, Test Movie and publishing

* AniCraft saves `.anicraft` documents, an open, documented format. It **opens** `.fla`, `.xfl`
  and XFL folders, as new, untitled documents, but can't save back to them. See
  [Opening Animate files](opening-animate-files.md).
* There is **no SWF** export. For the web, **Export HTML Canvas** writes one page that plays the
  movie, buttons and scripts in any browser. Video (with your own ffmpeg), GIF, animated PNG and
  WebP, Lottie, sprite sheets, SVG and PNG are there too, and **File › Publish** writes all the
  formats you choose at once. See [Exporting and publishing](exporting-and-publish.md).
* **Test Movie** (**Ctrl+Enter**) opens in its own window with an Output panel; **Esc** closes it.

## Not in AniCraft yet

Besides the tools and keys above, these are still to come:

* groups, saved workspaces and several open documents at once;
* motion tweens and bones from Animate documents;
* shape armatures, spring and joint translation for bones;
* the Brush's Paint Selection and Paint Inside modes, pressure curves and a brush library;
* per-character text styles and text on a path;
* full screen and zoom in Test Movie;
* shared libraries;
* SWF, AIR and other runtime targets.

The project's roadmap on GitHub keeps an honest, up-to-date list. If something you rely on is
missing, tell us on [GitHub issues](https://github.com/storytold/AniCraft/issues).
