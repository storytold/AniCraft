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
| Gradient Transform | **F** | Move, turn and stretch a gradient or bitmap fill |
| Width | **U** | Vary a line's thickness along it |
| Bone | **M** | Join symbol instances into a poseable armature |
| Pen | **P** | Draw precise curves point by point |
| Text | **T** | Type text on the stage |
| Pencil | **Shift+Y** | Draw freehand lines |
| Brush | **B** | Paint filled shapes |
| Eraser | **E** | Rub out lines and fills |
| Rectangle | **R** | Draw rectangles and squares |
| Oval | **O** | Draw ovals and circles |
| PolyStar | none | Draw polygons and stars |
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
it. To turn a drawing object back into merged drawing, select it and choose **Modify › Break
Apart** (**Ctrl+B**): it merges with the merged shapes it touches.

To change a drawing object's shape, **double-click** it (or select it and press **Ctrl+E**).
It opens in place, as a group does (see [Grouping](#grouping)): the rest of the stage dims, and
the bar above the stage shows **Scene 1 › Drawing Object**. Inside, its lines and fills are
merged drawing, cutting and joining only each other, and every tool works on them.
Double-click empty stage to go back. It stays one object; if your edits left it in several
pieces, it becomes a group of them.

As in Animate, what you've just drawn isn't selected, so no box gets in the way of the next
stroke. Switch to the Selection tool (**V**) to pick it up.

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
* **Ctrl-drag** (**⌘-drag** on a Mac) a line or the edge of a fill to add a **corner point**
  where you pressed and pull it into a peak: on a straight edge, both sides stay straight.
* **Drag an unselected corner or end** to move it. Every shape that shares the point moves
  with it, so the drawing stays joined.
* **Drag a fill** to move just that fill, without its outline. Drag anything already selected
  to move the whole selection.
* **Alt-drag** (**Option-drag**) a fill or a selection to drag a **copy** away. The original
  stays whole.
* Hold **Shift** while moving to keep the move horizontal, vertical or at 45°.

A small corner or curve mark beside the pointer shows what a drag will do. When you let go,
whatever you moved merges with what it lands on.

The marquee, **Shift**-click and **Edit › Select All** (**Ctrl+A**) work across every visible,
unlocked layer at once. **Edit › Deselect All** is **Ctrl+Shift+A**.

## Drawing shapes and lines

* **Rectangle** (**R**): drag to draw. **Shift** draws a square. Set **Corners** in the
  options for rounded corners.
* **Oval** (**O**): drag to draw. **Shift** draws a circle.
* **PolyStar**: drag from the centre to draw a polygon or a star. Its options (Animate's Tool
  Settings) are the **Shape**, **Polygon** or **Star**; **Sides** (or **Points**), 3 to 32;
  and for a star, the **Point size**, 0 to 1: near 0 the points are thin spikes, and at 1 the
  star is the polygon through its tips. It has no shortcut by default, because P is the Pen, as
  in Animate.
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
**Smoothing** to steady a shaky hand, and the **Mode**:

* **Paint Normal** covers lines and fills.
* **Paint Fills** leaves lines alone.
* **Paint Behind** paints only empty areas.
* **Paint Selection** paints only inside the selected fills. Select them with the Selection tool
  first; they stay selected, so you can keep painting.
* **Paint Inside** paints only inside the fill where your stroke starts, however far you go
  past its edge. Started on empty stage inside an outline, it fills only that empty area.

The last three never cover lines. With
a pen tablet, pressure sets the width, from 10 % to the full size.

**Texture and scatter.** **Texture** lets a paper grain show through the paint, like charcoal
on rough paper; the grain stays put on the stage, so strokes laid over each other share it.
**Scatter** throws dabs either side of the stroke and varies their size, for spatter and
stipple. Both are still ordinary vector shapes: they merge, erase, scale and export like any
other paint, and the same stroke always comes out the same. The **Charcoal** and **Spatter**
brushes in the Brush menu use them.

**Tilt.** Brushes can follow a pen's tilt: the tip turns with the way the pen leans (so a flat
nib follows your hand), and paints wider as it leans, up to twice as wide lying flat. A mouse
is never affected. Windows, macOS and Linux don't pass a pen's tilt on to AniCraft yet, so the
**Tilt** options (**Angle** and **Width**; the Paint Brush has **Width**) stay hidden in
Properties. They appear by themselves as soon as a pen reports its tilt. Meanwhile scripts and
assistants can paint with tilt: `draw.brush` takes points as `[x, y, pressure, altitude,
azimuth]` and `tilt_angle` and `tilt_width` (see [Working headless](working-headless.md)). The
**Charcoal** brush follows tilt when there is any.

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

Variable-width lines are merged drawing too. A line drawn across one, or the Eraser, cuts it
into pieces, and each piece keeps the widths it had there: a tapered line cut in the middle
gives two pieces, each tapering at its outer end.

For finer control, use the **Width** tool (**U**); the pointer becomes a crosshair. Hover over a
line: a white box on it shows where a drag adds a width point, and the line's points show
their handles. Drag on the line to make it wider or narrower there, drag a point's handle to
change it (**Alt**: one side only), or drag its dot to slide it along. Click a point and press
**Delete** to remove it.

## Fills and strokes

### Choosing colours

Set the colours for new artwork with the two squares at the bottom of the Tools panel (the
stroke's frame and the fill behind it), or in the drawing tool's options in Properties: the
**Fill** swatch and its on/off checkbox, and the **Stroke** swatch, its checkbox and its
width. The Pencil and the Line tool only draw lines, so they show no fill.

Click any colour swatch to open the colour picker. **Swatches**, the default, shows the 216
web-safe colours with greys and primaries beside them; a shade or highlight of a colour is
usually one cell away. The colours you mix or type are kept in the row below for this session.
**Mixer** sets any colour by hue, saturation and brightness (and alpha where the colour has
one). The hex field shows the colour as `#rrggbb` and takes one you type. A fill's picker
starts with the fill's type: **None** (no fill), **Solid**, **Linear**, **Radial** or
**Bitmap** (see [Gradients and bitmap fills](#gradients-and-bitmap-fills)); the stroke's picker
has **No colour** instead. In the Tools panel's picker, with shapes selected, the colour you
pick goes onto them too. Below the recent colours, the picker shows the document's own
swatches, and a fill's picker shows its gradient swatches under the fill type.

### Swatches

**Window › Swatches** keeps the colours and gradients you use in this document, as Animate's
Swatches panel does. They are saved with the document; a new document starts with greys, the
primaries and a few gradients.

* **Click a swatch** to apply it: to the selected shapes, and to the colour new drawing uses.
  It sets the **Stroke** or the **Fill**, whichever is active: click the Stroke or Fill square in
  the Tools panel or the Colour panel, or choose with **Swatches set the Stroke / Fill** at the
  top of the panel. A gradient always goes to the fill.
* **+** adds the active colour (the selected shape's fill or line, else the tool's), after the
  picked swatch. A gradient fill is added as a gradient swatch.
* **Drag** a swatch to move it. **Right-click** for Apply to Fill, Apply to Stroke, Duplicate,
  Delete, and its **Name**, which shows when you hover over it (otherwise its hex shows).
* The buttons under the swatches duplicate and delete the picked one. The **…** menu imports
  swatches, exports them, loads the default set and clears the panel (Clear keeps black and
  white). Every change can be undone.
* From the keyboard, **Tab** to a swatch: **Enter** applies it, **Delete** removes it and
  **Alt+Left/Right** moves it.

**Export Swatches** saves them as a small swatch file (`.json`) to import into other documents.
**Import Swatches** reads that file or a **GIMP palette** (`.gpl`, which Inkscape, Krita and
many palette sites also use); the swatches are added after yours. Adobe's `.aco`, `.ase` and
`.clr` swatch files are Adobe's own formats and don't open in AniCraft: export the palette
as GIMP `.gpl` from a tool that has it.

To recolour existing artwork, select it and change **Fill** and **Stroke** in Properties.
For transparency, use the colour effect's **Alpha** (see
[Colour effects](#colour-effects)).

### Line styles

Properties has Animate's line settings, for selected lines and for the tools' new lines:
**Style** (Solid, Dashed with dash and gap lengths, or Dotted with dot spacing), **Cap**
(None, Round or Square), **Join** (Miter, Round or Bevel) and **Miter**, how sharp a mitred
corner can get before it's bevelled.

### Gradients and bitmap fills

Choose **Linear**, **Radial** or **Bitmap** at the top of a fill's colour picker: the Tools
panel's Fill square, or the **Fill** swatch in Properties. With a shape selected it changes
that shape's fill; with a drawing tool in your hand, the Fill swatch in its options sets what
new shapes, the Paint Bucket and the Brush pour. A gradient's swatch shows its ramp.

A gradient shows as a bar with its colour stops under it:

* **Click the bar** to add a stop there. **Drag a stop** to move it, or drag it down off the
  bar to remove it. A gradient has 2 to 15 stops.
* Click a stop to pick it. Its **position** and **Remove** show under the bar, and the colour
  picker below them (Swatches, Mixer, the **hex** field and **alpha**) sets its colour and
  opacity. Type `#rrggbbaa` for a colour with opacity.
* From the keyboard, **Tab** to a stop, move it with the **arrow keys** (**Shift** for bigger
  steps) and press **Delete** to remove it. The bar itself adds a stop in the widest gap.
* **Overflow** sets what shows past the ends: **Extend** (the end colours carry on),
  **Reflect** (the ramp mirrors back and forth) or **Repeat**.
* **Linear RGB** mixes the colours in linear light, for brighter, more even blends.
* A radial gradient has a **Focal point**: where its ramp starts, from the centre.

Editing the stops never moves a gradient. A shape that gets a new kind of fill gets it fitted
to its own bounds: a linear gradient runs across its width, a radial one is centred and
touches its longer side, and a bitmap tiles from its top-left corner.

The **Gradient Transform** tool (**F**) changes where a gradient or bitmap fill lies. Click a
shape with one to show its handles:

| Handle | Linear | Radial | Bitmap | What a drag does |
|---|---|---|---|---|
| Round dot in the middle | ✓ | ✓ | ✓ | Moves the fill |
| Square | ✓ | ✓ | ✓ (left and bottom) | Changes its width (bitmap: width or height) |
| Circle with a dot | | ✓ | ✓ (corner) | Changes its size |
| Knob | ✓ | ✓ | ✓ | Turns it (**Shift**: 15° steps) |
| Triangle | | ✓ | | Moves the focal point along its axis |
| Diamonds | | | ✓ (top and right) | Skews it |

Each drag is one undo step. Gradients and bitmap fills from Animate files and SVG drawings
keep their overflow, Linear RGB, focal point and shape.

**Lock Fill** (in the Paint Bucket's and the Brush's options) pours the fill where it lies on
the stage instead of fitting it to each shape, so one gradient runs on across many shapes.
When you pick up a gradient or bitmap with the Eyedropper, Lock Fill turns on, so it stays
where it was.

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

Gradient fills and variable-width lines erase like any other: each piece that's left keeps its
part of the gradient where it was, and its widths along the line.

On the stage the Eraser leaves symbols and drawing objects alone, and the status bar says so.
To erase inside one, **double-click** it with the Selection tool (or select it and press
**Ctrl+E**): it opens in place, and the Eraser works on its own lines and fills. A symbol
changes everywhere it's used.

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

**Break Apart** turns text into shapes, a symbol instance into the artwork inside it, and a
drawing object into merged drawing.

### Envelope

**Modify › Transform › Envelope** bends the selected shapes and drawing objects. A mesh of
handles appears around them, with the Free Transform tool: **squares** move a corner (its
side's handles move with it) and **circles** bend a side. Each drag is one undo step, and
undo puts the mesh back where it was too. The envelope ends when you select something else or
pick another tool; bent merged drawing then joins what it lies on. The bend is part of the
shapes from then on, as in Animate, so there is nothing to delete (Animate's Delete Envelope
isn't needed). Symbols, text, groups and bitmaps can't be bent: break them apart first. Artwork with an Asset
Warp mesh can't either: remove its warp first.

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

Select some artwork and choose **Modify › Group** (**Ctrl+G**) to keep it together. A group
moves, scales and stacks as one, and it stays out of the Library. Groups can hold anything:
merged drawing, drawing objects, symbols, text and other groups.

* **Double-click** a group to edit what's inside it, right where it is. The rest of the stage
  dims, and the bar above the stage shows **Scene 1 › Group**. Every tool works inside. Merged
  drawing in a group only cuts and joins the shapes in the same group. Double-click empty stage,
  or click **Scene 1** in that bar, to go back.
* **Modify › Ungroup** (**Ctrl+Shift+G**) or **Break Apart** (**Ctrl+B**) puts the pieces back
  on the layer, exactly where they showed. Merged drawing then cuts and joins what it lands on.
* A group can have a colour effect, and it fades as one.

A group has one frame. To animate what's inside, make it a symbol with **Modify › Convert to
Symbol…** (**F8**), as described in [Symbols and the Library](symbols-and-library.md).

### Combine Objects

**Modify › Combine Objects** joins or cuts two or more selected shapes on one layer. The
topmost shape is the one on top in the stacking order.

* **Union** joins them into one. Shapes of one colour become one drawing object. Several
  colours, or lines, become a group of the merged shapes.
* **Intersect** keeps only the area every shape covers, in the top shape's fill and stroke.
* **Punch** cuts the top shape's area out of the shapes under it, and removes the top shape.
* **Crop** keeps only the parts of the shapes under it that are inside the top shape, and
  removes the top shape.

The top shape cuts with its fill, so it needs one. Break symbols, text and groups apart first
(**Ctrl+B**).

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
knockout and so on); the gradient ones have a colour ramp you click to add stops.

Each filter in the list has a row of its own:

* **The order matters**: filters apply from the top down, so a Glow above Adjust Color is
  greyed with the artwork, and one below it stays in colour. Drag a filter by its grip (the
  dots on the left) to move it; or click the grip and press **Alt+Up** or **Alt+Down**; or
  right-click the row for **Move Up** and **Move Down**.
* **The eye** switches a filter off without losing its settings, and back on. A filter that is
  off draws nothing anywhere: on the stage, in Test Movie and in every export.
* **The bin** removes the filter.

The **…** button next to **Add filter** has **Copy All Filters**, **Paste Filters** (adds the
copied filters after the instance's own; select another movie clip or button first),
**Enable All**, **Disable All** and **Remove All**. Right-click a filter's row for **Copy
Filter** to copy just that one.

**Filter presets** keep a list of filters under a name, for any document. Click the folder
button next to **Add filter**: **Save as Preset…** names the current list; click a preset's
name to apply it (it replaces the instance's filters, as in Animate; **Edit › Undo** takes it
back). **Rename Preset** and **Delete Preset** manage them (deleting asks first; artwork keeps
the filters it got). **Export Presets…** writes them to a small JSON file and **Import
Presets…** adds a file's presets to yours, to move them to another computer. Presets are kept
in your preferences.

**Tweening filters**: a classic tween changes filter values smoothly when both keyframes have
the same filters in the same order (switched-off ones count), as in Animate: a Blur from 0 to
20 is 10 halfway. On/off, Inner, Knockout and Quality stay as the first keyframe has them.
If the lists differ, the first keyframe's filters hold until the next keyframe.

**Blending** sets how the instance mixes with what's below it: Normal, Layer, Darken,
Multiply, Lighten, Screen, Overlay, Hard Light, Add, Subtract, Difference, Invert, Alpha and
Erase.

Filters and blend modes show on the stage and in exports. Some export formats can't carry
every filter or blend mode, and their reports say when one is left out (see
[Exporting and publishing](exporting-and-publish.md)).

To resize a rounded button or panel without stretching its corners, use **9-slice scaling**
on its symbol (see [Symbols and the Library](symbols-and-library.md#9-slice-scaling)).

## Rulers, grid and guides

* **View › Rulers** (**Ctrl+Alt+Shift+R**) shows rulers along the top and left of the stage,
  in pixels from the stage's top-left corner. They follow zoom and pan and mark where the
  pointer is.
* **Guides**: drag from the top ruler for a horizontal guide, or from the left ruler for a
  vertical one. With the Selection tool, drag a guide to move it, or drag it back onto a ruler
  to delete it. Each scene keeps its own guides, saved with the document.
  * **View › Guides**: **Show Guides** (**Ctrl+;**), **Lock Guides** (**Ctrl+Alt+;**), **Snap to
    Guides** (**Ctrl+Shift+;**), **Edit Guides** (**Ctrl+Alt+Shift+G**), **Add Guide** and
    **Clear Guides**.
  * **Edit Guides** sets the colour and snap accuracy. It also lists the scene's guides by
    position, so you can type a new position, delete one, or add one without a mouse.
* **Grid**: **View › Grid › Show Grid** (**Ctrl+'**), **Snap to Grid** (**Ctrl+Shift+'**) and
  **Edit Grid** (**Ctrl+Alt+G**). Edit Grid sets the spacing across and down, the colour, the
  snap accuracy (up to **Always snap**) and **Show over objects**. Without that option the grid
  shows under your artwork.
* **View › Snapping** has **Snap to Objects**, **Snap to Grid**, **Snap to Guides** and **Snap
  to Pixels**.

When several things are near, a point snaps in this order:

1. a corner or end of other artwork;
2. a guide;
3. a line or fill edge;
4. the grid;
5. a whole pixel.

Guides and the grid snap across and down separately, so a point can sit where a guide crosses a
grid line. Every drawing tool's points snap, and so do the Selection tool's moves (by the
corner or centre nearest the pointer) and Free Transform's handles. Hold **Shift** to constrain
without snapping. Rulers, the grid and guides never show in Test Movie or in exports.

## Not there yet

Not in AniCraft yet: gradient and bitmap strokes; swatch folders, bitmap swatches and the
swatches inside Animate files; moving the Envelope's handles from the keyboard; reading a
pen's tilt on Windows, macOS and Linux (the brushes' tilt settings are ready for it); bitmap
and art brushes; and Animate's ragged, stipple and hatched line styles (drawn
solid when you open an Animate file).
