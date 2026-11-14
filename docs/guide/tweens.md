# Tweens

A tween fills in the frames between two poses, so you draw the key moments and AniCraft does the
in-betweens. This chapter covers the three kinds of tween, easing, shape hints, the Motion
Editor and motion paths.

A note on keys: AniCraft uses **Ctrl** on Windows and Linux and **⌘** (Command) on macOS, and
**Alt** is **⌥** (Option) on macOS. So **Ctrl+Shift+H** is **⇧⌘H** on a Mac.

## Which tween should I use?

| Tween | Best for | On the timeline |
|---|---|---|
| **Classic tween** | Moving, scaling, rotating and fading an object between two keyframes. | Violet span |
| **Shape tween** | Morphing one shape into another: a square into a star, a blob into a splash. | Amber span |
| **Motion tween** | Animating an object property by property, with its own curves and a path you can bend. | Sky-blue span |

All three show the same everywhere: on the stage, in Test Movie and in every export.

You can create any of them from the **Insert** menu, from the right-click menu on the frames,
from the tween menu in the timeline header, or from **Properties › Frame**.

## Classic tweens

A classic tween runs from one keyframe to the next.

1. Put your object on a keyframe. A symbol instance works best (**F8** converts a selection to
   a symbol).
2. Go to a later frame and press **F6** to make a keyframe there.
3. Move, scale, rotate or fade the object on the new keyframe.
4. Click a frame in the span between them and choose **Insert › Create Classic Tween**.

Play it with **Enter**. If you change the object on a frame in the middle of a classic tween,
AniCraft adds a keyframe there for you.

To take a tween away, choose **Insert › Remove Tween**. The frames keep the first keyframe's
artwork.

## Easing

Easing makes motion speed up or slow down instead of moving at one steady pace. Pick an ease
for a classic or shape tween in either of these places:

* the ease menu in the timeline header (it appears while the playhead is in a tween);
* **Properties › Frame › Ease**, which also shows a small picture of the curve (click it to
  edit the curve).

The menu holds:

* **No Ease**: a steady pace.
* **Classic Ease**: one **Strength**, from −100 (starts slowly) to +100 (ends slowly).
* **Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce and Elastic**, each **In**, **Out** and
  **In Out**, drawn as small curves. An In ease starts slowly, an Out ease ends slowly, In Out
  does both. Quad to Quint get steeper in turn; Back overshoots a little and comes back;
  Bounce lands like a dropped ball; Elastic springs past the end and settles.
* The eases you saved in this document (see below).
* **Edit Ease…**, which opens the Custom Ease window.

### Custom eases

The **Custom Ease** window (Edit Ease…, the ✎ button in Properties, or **Edit Ease…** on the
span's right-click menu) shows the ease as a graph: time goes across, progress goes up from
0 % (the first keyframe) to 100 % (the next), and the curve may go above or below them for an
overshoot or a wind-up.

* Click the curve to add a point. The shape doesn't change until you move it.
* Drag a point, or the handles of the selected point, to shape the curve. Each drag is one
  undo step. A point can't pass its neighbours, so time never runs backwards.
* Double-click a point to make it smooth (handles in line) or a corner. Right-click a point to
  delete it. The two ends stay where they are.
* The selected point's **Time** and **Value** can be typed below the graph.
* With the graph focused, the keyboard does the same: `[` and `]` pick a point, the arrows move
  it (Shift moves ten times further), `S` makes it smooth or a corner, `+` adds a point after
  it, and Delete removes it.

Picking a preset and then dragging its curve turns it into a custom ease. **Reset** goes back
to No Ease.

**Ease each property separately** gives a classic tween one ease for each of **Position**,
**Rotation**, **Scale**, **Colour** and **Filters**: pick one at the top of the window and
shape its curve. Turn it off to use one ease for everything again. (Shape tweens have one
ease.)

### Saving and copying eases

Type a name beside **Save as** and click **Save** to keep the ease in the document. Saved eases
appear in every ease menu, for classic tweens and Motion Editor properties alike. A tween
given a saved ease gets a copy of its curve, so changing or deleting the saved ease later
doesn't change tweens that already use it. Click a saved ease's name under the buttons to edit
it, rename it or delete it.

**Copy Ease** and **Paste Ease** (in the window, or on the span's right-click menu) carry an
ease from one tween, property group or Motion Editor segment to another.

Motion tweens have their own easing per property: see [The Motion Editor](#the-motion-editor).

## Shape tweens

A shape tween morphs the shapes on one keyframe into the shapes on the next, even when they're
built quite differently (one blob into two, for example).

1. Draw a shape on a keyframe. Use plain shapes, not symbols.
2. Make a blank keyframe later on (**F7**) and draw a different shape there.
3. Click a frame between them and choose **Insert › Create Shape Tween**.

Fills, strokes, gradients and stroke widths blend as the shape changes. To switch a span between
a classic and a shape tween, use the tween menu in the timeline header or **Properties › Frame ›
Kind**; the easing is kept.

### Shape hints

Sometimes a morph twists or folds in a way you don't want. Shape hints tell AniCraft which
point on the first shape becomes which point on the last.

1. Put the playhead on the shape tween's first keyframe.
2. Choose **Modify › Shape › Add Shape Hint** (**Ctrl+Shift+H**). A lettered marker, starting
   at **a**, appears at the centre of the shape.
3. Drag the marker onto an edge of the shape. It snaps to the nearest edge.
4. Go to the last keyframe and drag the same letter onto the matching edge there.
5. Add more hints the same way, up to **z**, working round the shape in order.

The colours tell you where each hint stands: rose until it's placed, amber once it's placed on
the first keyframe, and mint on the last. Right-click a hint to remove it, or choose **Modify ›
Shape › Remove All Hints**. **Ctrl+Z** undoes either.

Not there yet: a **View** menu switch to hide the hints, and hints inside a symbol you're editing
in place.

## Motion tweens

A motion tween animates one object, and gives every property its own timing. There are no
keyframes in the middle of the span: instead, each property (X, Y, scale, rotation, skew, alpha
and the colour effects) has **property keyframes** wherever you need them.

### Creating a motion tween

1. Place an object on a keyframe.
2. With the playhead on that span, choose **Insert › Create Motion Tween**.

If the keyframe holds several objects, they become one graphic symbol first. If the span is only
one frame long, it grows to a second's worth of frames. Both work as they do in Animate.

### Adding property keyframes

Move the playhead to any frame inside the span and change the object: move it, scale it, rotate
it or fade it. AniCraft keys just the properties you changed, at that frame. A diamond on the
span marks each frame with property keyframes.

Press **F6** inside the span to key every property at once.

## The Motion Editor

The Motion Editor shows a motion tween's properties as curves you can shape.

![The Motion Editor with a property's curve and ease handles](images/motion-editor.png)

Open it with the **Motion Editor** tab beside **Timeline**, **Window › Motion Editor**, or a
double-click on a motion span.

On the left is the list of properties, with each one's value at the playhead.

* Drag a value to change it and key it at the playhead.
* Click the diamond (◇) next to a property to add a keyframe there, or to remove the one that's
  there.
* **Add Colour Effect** lists a colour effect style's properties (such as brightness or tint) so
  you can key them.

On the right is the selected property's curve:

* drag a key up or down to change its value;
* drag a key sideways to move it in time (see below);
* drag the two handles of a segment to shape its ease (overshoot is allowed, for a bounce or a
  wind-up);
* double-click to add a key;
* click to move the playhead.

Under **Ease**, pick a preset or a saved ease for the segment at the playhead, or click ✎ to
shape it in the Custom Ease window. Eases that one curve can't draw (Bounce, Elastic, custom
eases of several points) show no handles in the graph: shape them in the window. **Same for
all** gives every property's segment there the same ease. Each drag is one undo step.

**Remove Tween** takes the motion tween away, leaving the object as it is on the span's first
frame.

### Animating filters

A movie clip or button with filters (see **Properties › Filters**) lists each filter in the
Motion Editor, below the colour effect, with its numbers under its name: **Blur X** and **Blur Y**,
**Strength**, **Angle** and **Distance** for the shadows, glows and bevels, and **Brightness**,
**Contrast**, **Saturation** and **Hue** for Adjust Color. They key, curve, ease and retime like
any other property, so a blur can sharpen with an ease-out and then pulse.

You can also change a filter in **Properties › Filters** with the playhead inside the tween: the
number you changed gets a key there, as moving the object does. Reordering or removing filters
takes their keys along with them. A filter's colours, quality and switches aren't animated yet;
they stay as the object has them.

### Moving keys in time

Drag a key **sideways** to move it to another frame. It snaps to whole frames, carries its value
and ease with it, and never passes the key before or after it on the same property (it stops
next to that key). A drag goes either sideways or up and down, depending on which way you first
move the pointer, so retiming never changes a value by accident.

To move several keys together:

* **Shift-click** (or Ctrl-click) keys to pick them. The pick stays when you switch to another
  property, so you can pick keys of X, Y and rotation and move them as one.
* The **All** lane under the frame numbers has a diamond on every frame that holds a key. Click
  one to pick every property's keys on that frame, then drag it to move them all.

With the curve focused (click it, or Tab to it), **Left** and **Right** move the picked keys one
frame; with **Shift**, ten frames. Each drag, and each key press, is one undo step.

On a motion path, a frame pinned to a path point keeps its X and Y keys together: moving one
moves both, and the pin moves with them.

## Retiming tweens on the timeline

* **Stretch a tween**: drag the right edge of a span. On a motion tween its property keyframes
  stretch in proportion, as in Animate: a key halfway through stays halfway through. Hold
  **Shift** while you let go to keep the keys on their frames instead. The readout beside the
  edge shows the new length and what happens to the keys. Later frames on the layer move with
  the edge. Classic and shape tweens get longer or shorter the same way.
* **Move a tween**: click a motion tween span to pick the whole span (Ctrl-click picks a single
  frame), then drag it. A selection that covers whole classic or shape tween spans moves the
  same way. The span keeps its length and its tween, and replaces what it lands on. The frames
  it leaves become empty.
* **Copy a tween**: hold **Alt** while you drag. The original stays where it was.
* **Split Motion**: right-click a motion tween on any frame after its first and choose **Split
  Motion**, or use **Modify › Timeline › Split Motion**. You get two tweens, and the object
  moves exactly as it did before.
* **Join Motion**: put the playhead on a motion tween that has another motion tween of the same
  object right after it, and choose **Join Motion** from the right-click menu or **Modify ›
  Timeline**. To join more than two, select frames across all of them first. Tweens you split
  join back exactly as they were. Other tweens keep how the object looks on every frame. Join
  Motion doesn't join two tweens with their own motion paths yet, and it won't join a keyframe
  that has a sound or actions on it (move those first).

## Motion paths

When you select a motion tween's object, or a frame of its span, the stage shows the path the
object travels, with a dot for every frame. A straight path appears as soon as the object
moves.

### Bending the path

* With the **Selection** tool (**V**), drag the path to bend it, or drag an end to change where
  the motion starts or finishes.
* With the **Subselection** tool (**A**), drag the path's points and their handles. Smooth points
  stay smooth; hold **Alt** to move one handle on its own.

The object follows the path at the pace its X and Y curves set, so your eases carry over.

### Adding and removing path points

* With the Subselection tool, double-click the path to add a point. Nothing moves when you do.
* With the **Pen** tool (**P**), **Alt**-click the path to add a point.
* **Shift**-click to select several points, then press **Delete** to remove them. The path refits
  through the rest. The two ends always stay, because that's where the motion starts and ends.
* Right-click a point for **Make Smooth**, **Make Corner** and **Straighten Path**.

### Moving the whole path

**Ctrl**-drag the path or its object to move the whole path, or type a new position in
**Properties › Frame › Path start**. The timing stays exactly as it was.

### Orient to path and Straighten

In **Properties › Frame**, turn on **Orient to path** to make the object turn as the path turns,
like a car following a road. **Straighten** goes back to a straight path.

### Points pinned to frames

When you move the object at a frame in the middle of the span, the path gets a point there that
is **pinned** to that frame: the object passes exactly through that point on that frame, however
else you reshape the path. After the last pinned point, the path carries on to the new place.

* Pinned points show on the stage as diamonds with their frame number. You can drag them with the
  Selection tool.
* With the Subselection tool, right-click any point and choose **Pin to Frame** to pin it to the
  playhead's frame, or **Unpin from Frame** to let it shape the path only.
* **Properties › Frame** has **Pin Here** and **Unpin** for the playhead's frame.
* Pressing **F6** inside the span pins a point where the object is.

The path's two ends are always pinned. In the Motion Editor, a pinned frame's X and Y keys are
blue; dragging one moves its point. On the timeline, pinned frames have a larger diamond.

Not there yet: retiming a pin by dragging its key, two pins on one point, selecting path points
with a marquee, and nudging or transforming a whole path.

## Following a drawn path with a classic tween

Classic tweens can follow a path too, using a motion guide layer. See
[Masks, guides, camera and bones](masks-guides-camera-bones.md).

## Tweens in Animate files

When you open an Animate document, its classic tweens (with their eases, custom eases of any
number of points and eases set per property included), shape tweens and shape hints come
across. Motion tweens and their motion paths don't come across from Animate files
yet. See [Opening Animate files](opening-animate-files.md).

## Next steps

* Layers, keyframes and onion skin: [The timeline](timeline.md).
* Make your movie respond to clicks: [Interactivity](interactivity.md).
