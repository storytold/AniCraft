# Drawing

AniCraft's drawing tools work the way Animate's do: shapes on a layer cut into and join each
other, the Selection tool bends lines, and the Paint Bucket fills what your lines enclose.
This chapter covers every tool, the two ways of drawing, and how to colour, arrange and
transform your artwork.

Shortcuts are written the Windows and Linux way. On macOS, use ⌘ (Command) for Ctrl and ⌥
(Option) for Alt: **Ctrl+B** is **⌘B**.

## The tools

The Tools panel on the left holds every tool, in four groups. Hover a tool to see its name,
key and a short explanation. The status bar repeats the explanation while the tool is in
your hand.

| Tool | Key | What it does |
|---|---|---|
| Selection | **V** | Select, move and bend artwork |
| Subselection | **A** | Edit points and curve handles |
| Free Transform | **Q** | Scale, rotate and move |
| Width | **U** | Vary a line's thickness along it |
| Bone | **M** | Join symbol instances into a poseable armature |
| Pen | **P** | Draw precise curves point by point |
| Text | **T** | Type text on the stage |
| Pencil | **Shift+Y** | Draw freehand lines |
| Brush | **B** | Paint filled shapes |
| Eraser | **E** | Rub out lines and fills |
| Rectangle | **R** | Draw rectangles and squares |
| Oval | **O** | Draw ovals and circles |
| PolyStar | none | Draw stars |
| Line | **N** | Draw straight lines |
| Paint Bucket | **K** | Fill enclosed areas, or recolour fills |
| Ink Bottle | **S** | Give a shape the current stroke |
| Eyedropper | **I** | Pick up a fill or line style |
| Camera | **C** | Pan, zoom and turn the scene's camera |
| Hand | **H** | Pan the view |
| Zoom | **Z** | Zoom the view |

While a drawing tool is in your hand, its options appear at the top of **Properties**: fill
and stroke, line style, and the tool's own settings. You can change any of these keys in the
shortcut editor (**F1**).

The Bone and Camera tools are covered in
[Masks, guides, camera and bones](masks-guides-camera-bones.md), and the Text tool in
[Text](text.md).

## Merge drawing and drawing objects

AniCraft has two ways of drawing, as Animate does.

**Merge drawing** is the default. Lines and fills you draw become part of one connected
drawing on that layer:

* shapes that touch cut into and join each other;
* a fill drawn on top replaces what it covers, or joins a fill of the same colour;
* lines cut each other where they cross, and a line right across a fill splits it in two;
* a fill hides the lines under it;
* points closer than a fifth of a pixel become one.

This is the classic, "hand-drawn" way of working: sketch overlapping lines, delete the bits
you don't want, and fill the spaces.

**Object Drawing** keeps each shape to itself. Shapes drawn this way are separate objects
that never cut or merge, and they're easy to move, stack and edit one at a time.

To switch between them, press **J**, or tick **Object drawing** in the tool's options in
Properties. The setting applies to the Rectangle, Oval, PolyStar, Line, Pen, Pencil and Brush
tools.

The Eraser only erases merged drawing. Drawing objects, symbols and text are left alone by
it.

### Selecting merged drawing

With the **Selection** tool (**V**) on merged drawing:

* **Click** a fill or one stretch of line to select just that. Selected parts show a dot
  pattern rather than a box.
* **Double-click** a line to select every line joined to it, or a fill to select the fill and
  its outline.
* **Drag a marquee** to select the parts of fills and lines under it, cut at the marquee's
  edges, plus any objects wholly inside it. **Shift** adds more. If you click away without
  changing anything, the drawing is whole again.
* **Drag an unselected line** or the edge of a fill to **bend** it into a curve through the
  pointer. Its ends stay put.
* **Drag an unselected corner or end** to move it. Every shape that shares the point moves
  with it, so the drawing stays joined.
* **Drag a fill** to move just that fill, without its outline. Drag anything already selected
  to move the whole selection.

A small corner or curve mark beside the pointer shows what a drag will do. When you let go,
whatever you moved merges with what it lands on.

The marquee, **Shift**-click and **Edit › Select All** (**Ctrl+A**) work across every visible,
unlocked layer at once. **Edit › Deselect All** is **Ctrl+Shift+A**.

## Drawing shapes and lines

* **Rectangle** (**R**): drag to draw. **Shift** draws a square. Set **Corners** in the
  options for rounded corners.
* **Oval** (**O**): drag to draw. **Shift** draws a circle.
* **PolyStar**: drag from the centre to draw a star. **Points** in the options sets how many
  points it has (3 to 32). It has no shortcut by default, because P is the Pen, as in Animate.
  Polygons aren't available yet.
* **Line** (**N**): drag to draw. **Shift** keeps the angle to steps of 45°.
* **Pencil** (**Shift+Y**): drag to draw freehand. Choose a mode in the options:
  **Smooth** evens out your line into flowing curves, **Straighten** turns it into straight
  segments and clean corners, and **Ink** keeps it close to what you drew.

### The Pen

The **Pen** (**P**) draws precise Bézier curves. Click to place corner points, or drag to pull
out smooth handles (**Alt** moves one handle on its own). Click the first point to close the
shape; **Enter**, **Esc** or a click on the last point finishes an open line. **Backspace**
takes back the last point, and **Alt**-click on a line or a selected shape adds a point to it.

### The Brush

The **Brush** (**B**) paints filled shapes in the fill colour: the shape is exactly the area
the tip sweeps. A click paints a single dab. The options set the **Size** (or press **[** and
**]**), **Shape** (Round or Square), **Angle** and **Roundness** for calligraphic tips,
**Smoothing** to steady a shaky hand, and the **Mode**: **Paint Normal** covers lines and
fills, **Paint Fills** leaves lines alone, and **Paint Behind** paints only empty areas. With
a pen tablet, pressure sets the width, from 10 % to the full size.

**Snapping while you draw**: with **Snap to Objects** on (the magnet under the tools, or **Ctrl+Shift+/**), the Line,
Rectangle, Oval, PolyStar and Pen tools snap their points to nearby points, lines and fill
edges, within 10 screen pixels. A ring shows where a point will snap.

## Editing points with Subselection

The **Subselection** tool (**A**) shows and edits the points of a shape:

* **Click** a shape to see its points (on merged drawing, those of everything connected to
  it). Click a point to select it, **Shift**-click to add or remove, or marquee on empty stage.
* **Drag** a point to move it with the other selected points, and every shape sharing it.
* **Drag a handle** to change the curve. A smooth point keeps its two handles in line, and
  **Alt** moves one alone.
* **Double-click** an edge to add a point. The curve keeps its shape.
* **Delete** removes the selected points, joining the shape across them. A point where three
  or more lines meet stays, and the status bar says why.
* **Right-click** a point for **Make Smooth** or **Make Corner**.
* The **arrow keys** nudge the selected points by one screen pixel (**Shift**: ten).

## Variable-width lines

A line's thickness can change along its length. To give a line a profile, select it and pick
one under **Width** in Properties: Uniform, Taper both ends, Taper start, Taper end, Bulge or
Pinch. Set a profile in a drawing tool's options to make new lines start that way.

For finer control, use the **Width** tool (**U**). Hover over a line to see its width points.
Drag a point's handle to make the line wider or narrower there (**Alt**: one side only), drag
its dot to slide it along, or drag anywhere on the line to add a point. Click a point and
press **Delete** to remove it.

## Fills and strokes

### Choosing colours

Set the colours for new artwork in the drawing tool's options in Properties: the **Fill**
swatch and its on/off checkbox, and the **Stroke** swatch, its checkbox and its width. The
two squares at the bottom of the Tools panel show the current fill and stroke.

To recolour existing artwork, select it and change **Fill** and **Stroke** in Properties.
For transparency, use the colour effect's **Alpha** (see
[Colour effects](#colour-effects)).

### Line styles

Properties has Animate's line settings, for selected lines and for the tools' new lines:
**Style** (Solid, Dashed with dash and gap lengths, or Dotted with dot spacing), **Cap**
(None, Round or Square), **Join** (Miter, Round or Bevel) and **Miter**, how sharp a mitred
corner can get before it's bevelled.

### Gradients and bitmap fills

AniCraft draws linear and radial gradients and bitmap fills, and keeps them from Animate
files and SVG drawings you import. There isn't a gradient editor for fills yet, so you can't
make a new gradient from scratch in AniCraft. You can reuse one, though: pick it up with the
Eyedropper and pour it with the Paint Bucket. A picked gradient stays where it was on the
stage, so it carries on across every shape you pour it into.

### Paint Bucket, Ink Bottle and Eyedropper

The **Paint Bucket** (**K**) fills an area enclosed by lines, or recolours a fill you click.
On merged drawing the new fill runs exactly up to the lines and fill edges, and joins any
touching fill of the same colour. If your outline has small gaps, choose **Gap size** in the
options: Don't close gaps, Close small gaps, Close medium gaps or Close large gaps. If an
area is open, the status bar says so instead of flooding the stage.

The **Ink Bottle** (**S**) gives the shape you click the current stroke: its colour, width,
width profile and line style.

The **Eyedropper** (**I**) picks up a style. Click a **fill** to pick its colour, gradient or
bitmap; the tool then becomes the Paint Bucket, ready to pour it. Click a **line** to pick its
colour, width, profile and line style; the tool then becomes the Ink Bottle. With shapes
selected, the picked style goes straight onto them. **Shift**-click picks into the other
swatch only.

### Eraser

The **Eraser** (**E**) rubs out merged lines and fills on the active layer. It uses the same
tip as the Brush (**[** and **]** change its size). Choose a **Mode** in the options: **Erase
Normal**, **Erase Fills**, **Erase Lines**, **Erase Selected Fills** or **Erase Inside**,
which erases only the fill where your stroke starts.

Tick **Faucet** to delete a whole fill or line with a single click. **Erase Everything…**, or
a double-click on the Eraser tool, clears every visible, unlocked layer at the current frame.
It asks first, and you can undo it.

The Eraser can't erase inside symbols, or lines with a width profile, yet.

## Transforming artwork

With the **Free Transform** tool (**Q**), drag a corner or edge to **scale** (**Shift** keeps
the proportions, **Alt** scales from the centre), just outside a corner to **rotate**
(**Shift** snaps to 15°), or inside the box to **move**. You can also type exact
**Position**, **Scale** and **Rotation** values in Properties.

| Command | Menu | Key |
|---|---|---|
| Rotate 90° CW | **Modify › Transform** | **Ctrl+Shift+9** |
| Rotate 90° CCW | **Modify › Transform** | **Ctrl+Shift+7** |
| Flip Horizontal, Flip Vertical | **Modify › Transform** | none |
| Remove Transform | **Modify › Transform** | **Ctrl+Shift+Z** |
| Break Apart | **Modify** | **Ctrl+B** |

**Break Apart** turns text into shapes, and a symbol instance into the artwork inside it.

## Copying, arranging and aligning

### Clipboard and duplicates

**Edit** has **Cut** (**Ctrl+X**), **Copy** (**Ctrl+C**), **Paste in Center** (**Ctrl+V**),
**Paste in Place** (**Ctrl+Shift+V**), **Duplicate** (**Ctrl+D**) and **Clear** (**Delete**
or **Backspace**). Copied artwork keeps its layers, and symbols travel with it to another
document.

### Stacking order

Things on the same layer stack one above another. **Modify › Arrange** has **Bring to Front**
(**Ctrl+Shift+Up**), **Bring Forward** (**Ctrl+Up**), **Send Backward** (**Ctrl+Down**) and
**Send to Back** (**Ctrl+Shift+Down**). To put something above everything on another layer,
move it to a higher layer (see [Timeline](timeline.md)).

### Align and distribute

Use the **Align** panel in the right-hand dock, or **Modify › Align**: **Left**
(**Ctrl+Alt+1**), **Horizontal Center** (**Ctrl+Alt+2**), **Right** (**Ctrl+Alt+3**), **Top**
(**Ctrl+Alt+4**), **Vertical Center** (**Ctrl+Alt+5**) and **Bottom** (**Ctrl+Alt+6**).
**Distribute Horizontal Centers** and **Distribute Vertical Centers** space three or more
things evenly. Tick **To stage** in the Align panel (or press **Ctrl+Alt+8**) to align to the
stage instead of the selection. A single shape always aligns to the stage. Align works across
layers.

### Grouping

There's no Group command (Animate's **Ctrl+G**) yet. To keep shapes together, either draw
them with Object Drawing on, or select them and make them a symbol with **Modify › Convert to
Symbol…** (**F8**), as described in [Symbols and the Library](symbols-and-library.md).

## Colour effects

Every shape, text block and symbol instance has a **Colour effect** in Properties, with the
same styles as Animate: **None**, **Brightness** (−100 % black to 100 % white), **Tint** (a
colour and an amount), **Advanced** (a multiplier and an offset for red, green, blue and
alpha) and **Alpha** (transparency). Switching style keeps the look where it can. On symbol instances, classic and motion tweens
animate the colour effect too (see [Tweens](tweens.md)).

## Filters and blend modes

Movie clip and button instances can also have **Filters** and **Blending**, as in Animate.
Convert your artwork to a movie clip symbol first (see
[Symbols and the Library](symbols-and-library.md)).

**Filters**: choose **Add filter** under Filters in Properties, and stack as many as you like:
**Drop Shadow**, **Blur**, **Glow**, **Bevel**, **Gradient Glow**, **Gradient Bevel** and
**Adjust Color**. Each has its own settings (blur, strength, quality, colour, angle, distance,
knockout and so on); the gradient ones have a colour ramp you click to add stops. **Remove**
takes a filter off.

**Blending** sets how the instance mixes with what's below it: Normal, Layer, Darken,
Multiply, Lighten, Screen, Overlay, Hard Light, Add, Subtract, Difference, Invert, Alpha and
Erase.

Filters and blend modes show on the stage and in exports. Some export formats can't carry
every filter or blend mode, and their reports say when one is left out (see
[Exporting and publishing](exporting-and-publish.md)).

To resize a rounded button or panel without stretching its corners, use **9-slice scaling**
on its symbol (see [Symbols and the Library](symbols-and-library.md#9-slice-scaling)).

## Not there yet

Not in AniCraft yet: rulers, a grid and guides; grouping (**Ctrl+G**); a gradient editor and
a Colour panel with swatches; PolyStar's Polygon option; the Paint Selection and Paint Inside
brush modes, brush libraries and tilt; erasing inside symbols; and Animate's ragged, stipple
and hatched line styles (drawn solid when you open an Animate file).
