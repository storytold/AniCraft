# 0076: Rulers, grid and guides

Status: accepted (M1, first slice)

## Context

Animate users rely on rulers, a grid and guides every day: to place artwork on a layout, line up
characters with a horizon, keep a safe area, or work on a tile grid. The user guide listed them
as missing. AniCraft already snapped to artwork (Snap to Objects, ADR 0041, 0042, 0045), but
that snapping lived in three places (`raw.move_point`, `element.translate`, `draw.snap`), each
gated on one switch.

Behaviour is learned from Adobe's public help pages for Animate (View › Rulers, Grid, Guides,
Snapping and the Edit Grid / Edit Guides dialogs) and from observation; no Adobe code or assets.

## Decision

**Model.** Each scene has its guides (`Scene::guides: Vec<Guide>`, `{axis: horizontal|vertical,
at}` in stage pixels from the stage's top-left corner), as Animate keeps them per scene. How the
grid and guides look is the document's (`Document::aids`: grid spacing x/y, grid colour, *show
over objects*, guide colour), so a file reopens, and opens on another computer, with the same
layout. Both are `#[serde(default)]` and skipped when empty or default: older files open
unchanged and files without guides write nothing new. Sanitizing keeps at most 1,000 guides per
scene within ±100,000 px and the grid's spacing within 1–4,096 px.

Whether the rulers, grid and guides **show and snap** (and Lock Guides and the two snap
accuracies) is the editor's working preference (`Session::aids: ViewAids`), like Snap to
Objects: never in the document or its undo history, kept between sessions in the preferences.
Toggling the grid is not an edit, and Ctrl+Z after it undoes the last drawing, not the toggle.

**Commands** (`view.*`, `cmds_view.rs`), so agents, the CLI and MCP can do everything:
`view.rulers`, `view.grid_show`, `view.grid_snap`, `view.grid_set` (Edit Grid),
`view.guides_show`, `view.guides_lock`, `view.guides_snap`, `view.guides_set` (Edit Guides),
`view.guide_add`, `view.guide_move`, `view.guide_delete`, `view.guides_clear`,
`view.snap_pixels`, and the query `view.aids`. Guide edits and look changes are undoable
document edits; a ruler-to-stage drag is one `view.guide_add`; a guide drag is one gesture (one
undo step, a delete included). While guides are locked, moving or deleting one by one is
refused; adding and Clear Guides still work. Plug-ins may read and edit a scene's guides, not
change what shows and snaps.

**One snap engine** (`crates/engine/src/snap.rs`). Every tool that snaps asks the same
function with a radius of 10 screen pixels at the current zoom, in this order of priority:

1. an object's **point** (corner or end) within the radius: both coordinates;
2. a **guide**, each coordinate on its own (a vertical guide takes x, a horizontal one y; both
   together give their crossing), within the radius times the guide accuracy;
3. an object's **line** or fill edge, only when no guide took either coordinate;
4. the **grid**, each coordinate not yet taken, within the radius times the grid accuracy;
5. a whole **pixel** (Snap to Pixels), each coordinate not yet taken.

Accuracy: *Must be close* ½×, *Normal* 1×, *Can be distant* 2×, and for the grid only *Always
snap*. Points come first because snapping a corner onto a corner is the most specific intent;
guides beat lines because they are placed on purpose; the grid and pixels are the background.
Per-axis snapping lets a point sit on a guide and a grid line at once. Shift (constrain) asks
with no radius, so nothing snaps. Guides and the grid are in stage space; while a symbol is open
in place the point is mapped to the stage and back, so it lands where the stage shows the guide.

Who snaps: the Line, Rectangle, Oval, PolyStar and Pen points (`draw.snap`), a dragged raw
corner or end (`raw.move_point`), the Selection tool's move (`element.translate`: the
selection's anchor or **bounding-box corner or centre** nearest the pointer, so symbols, text
and bitmaps snap too), and Free Transform's scale handles and move (`draw.snap` with `moving`,
which leaves the selection out of the targets). The snap ring shows where, except for a pixel
snap.

**Drawing.** All of it is the shell's overlay, never in a picture or an export. The rulers are
strips along the stage view's top and left (theme `RULER`), in stage pixels whatever symbol is
open, with the pointer marked; major steps are chosen to keep labels ≥ 56 screen points apart.
Guides span the whole view, pasteboard included. The grid covers the stage. *Under objects*
(the default, as in Animate) the stage's tiles render without the stage colour, which is painted
under the grid instead; on the CPU and GPU stage alike (the pasteboard already uses transparent
tiles, #256). Blend modes that blend against the stage colour then blend against transparency
while the grid shows under the artwork; turning on *Show over objects* or hiding the grid gives
the exact picture back. The grid and guides hide while previewing through the camera.

**Interaction.** Drag from the top ruler for a horizontal guide, from the left one for a
vertical guide; with the Selection tool drag a guide to move it, and let go over a ruler or off
the stage to delete it (positions from a drag are whole pixels). Edit Guides lists the scene's
guides with their positions, a Delete per guide and an Add row, so guides are created, moved and
deleted from the keyboard; View › Guides › Add Guide opens it on the Add row. Each dialog works
on a copy and applies on OK as one undo step.

## Consequences

* Snapping has one implementation and one documented order; new tools call `draw.snap`.
* `Document` grows by one `Arc` (kept small for the recovery writer's enum).
* Guides inside a symbol are the scene's, in stage space, not the symbol's own guides as in
  Animate.

Not in this slice: units other than pixels, a ruler origin other than the stage's corner,
symbol-level guides, guides and grid settings from XFL/FLA files, Snap Align (alignment hints to
other objects' edges), Edit Snapping's dialog, snapping while rotating, the Brush and Pencil
snapping, and a pixel grid drawn at high zoom.
