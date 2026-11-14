# Masks, guides, camera and bones

Some layers do a special job. This chapter covers mask layers, guide layers and motion guides,
the camera layer, bones for posing a character, and Asset Warp for bending artwork with pins.

A note on keys: AniCraft uses **Ctrl** on Windows and Linux and **⌘** (Command) on macOS, and
**Alt** is **⌥** (Option) on macOS. So **Ctrl+Z** is **⌘Z** on a Mac.

## Mask layers

A mask layer shows the layers under it only where the mask has fills: a spotlight, a window, a
wipe, or text that reveals a picture.

### Making a mask

1. Put the artwork you want to reveal on a layer.
2. Make a new layer right above it and draw the mask shape there, with a fill.
3. Right-click the mask layer and choose **Mask** (or choose **Modify › Timeline › Mask**).

The layer below becomes **masked**: it's indented under the mask, which gets a stencil icon. To
mask more layers with the same mask, right-click each layer right under the group and choose
**Masked**, or move it in under the mask with the up and down buttons. Moving a layer out of the
group takes it out of the mask.

### How masks behave

* Only the mask's **fills** count. Strokes, and the mask artwork's opacity, are ignored.
* The mask always applies, on the stage and in every export, so what you see is what you get.
  If you're coming from Animate, there's no need to lock both layers to preview it.
* While the mask layer is unlocked, its shapes' edges show in the layer colour, so you can select
  and reshape them. Lock the mask layer to hide the edge.
* Parts of the masked artwork that are hidden can't be picked on the stage.
* Hiding the mask layer turns the mask off.
* The mask layer's artwork can have keyframes and tweens like any other layer, for moving
  spotlights and wipes.
* A mask moves with its whole group when you reorder layers. Masks work inside folders and inside
  symbols too.

### Soft masks

A mask doesn't have to have hard edges. Double-click the mask layer's icon (or right-click it and
choose **Properties…**) and pick a **Mask type**:

* **Shape**: the mask's fills reveal fully, whatever their colour or opacity. This is what
  Animate's masks do, and it's the default.
* **Alpha**: the mask's opacity reveals. Fill the mask with a gradient that goes to transparent
  and the artwork fades out with it; a shape at 50 % opacity shows the artwork at half strength.
  Strokes, bitmaps and colour effects on the mask count too.
* **Luminance**: the mask's brightness reveals: white shows, black hides, greys show partly.
  This is how SVG masks work.

Soft masks show on the stage and in Test Movie, and export to SVG, Lottie (as track mattes) and
HTML.

## Guide layers

A **guide** layer holds artwork that helps you draw, such as a layout sketch, a grid or safe
areas. It shows on the stage but never in Test Movie or an export. Right-click a layer and
choose **Guide** to make it one, and choose it again to turn it back into a normal layer.

The built-in templates use a locked guide layer for their layout guides.

## Motion guides

A **motion guide** is a guide layer with a path that the classic tweens on the layer below
follow.

1. Make a classic tween on a layer (see [Tweens](tweens.md)).
2. Right-click that layer and choose **Add Classic Motion Guide** (or choose **Insert › Timeline ›
   Add Classic Motion Guide**). A guide layer appears above it, and your layer becomes
   **guided**.
3. On the guide layer, draw the path with the **Pen**, **Pencil** or **Line** tool.

The tweened artwork now travels along the line, from the point nearest its start keyframe to
the point nearest its end keyframe. To make it turn as the path bends, click a frame of the
tween and turn on **Orient to path** in **Properties › Frame**.

To guide another layer by the same path, right-click a layer right under the guide and choose
**Guided**. The path shows on the stage while you work and never appears in exports.

For motion tweens, you don't need a guide layer: they have their own motion path, which you
bend right on the stage. See [Motion paths](tweens.md#motion-paths).

## The camera

The camera lets you pan, zoom and turn the view of the scene, as a film camera would, without
moving your artwork.

### Adding a camera

Do any of these:

* click the camera button in the timeline header;
* choose **Insert › Timeline › Add Camera**;
* pick the **Camera** tool (**C**). If the scene has no camera yet, it adds one.

A **Camera** layer appears at the top of the timeline. On the stage, an orange outline shows the
part of the scene the movie will show.

### Moving the camera

With the Camera tool:

* drag on the stage to pan;
* drag the round handle above the frame to turn the camera;
* use the mouse wheel to zoom.

Each drag is one undo step. For exact values, use **Properties › Camera**: **Position**,
**Zoom** (5 to 2,000 %), **Rotation**, and a **Colour effect** over the whole view: **Brightness**,
**Tint** or **Advanced** (red, green and blue multipliers and offsets), as on a symbol instance.
The camera has no opacity, so there is no Alpha style. Colour effects tween with the camera.
**Reset** puts the camera back to the plain stage: centred, 100 %, no turn and no colour effect.

### Animating the camera

The camera animates like any object. Add keyframes on the Camera layer, change the camera on
each, and create a classic or motion tween between them. In the Motion Editor, the camera's zoom
is **Scale X**.

The Camera layer holds only the camera. Draw your artwork on other layers.

### Layer depth

Once the scene has a camera, each layer can have a **depth**: how far behind the stage it sits,
in pixels (a negative depth puts it in front, down to −500). The camera sees the layers as a real
camera would (a pinhole camera 1,000 px from the stage): when it pans, a deeper layer moves less;
when it zooms in, a deeper layer grows less. Deeper layers are drawn behind nearer ones; layers at
the same depth keep their timeline order. With the camera at rest, every layer looks just as you
drew it, and a layer at depth 0 behaves exactly as it always did.

Open **Window › Layer Depth** to see the layers listed by depth, nearest first. Drag a layer's
handle along its track (left is nearer, right is farther) or type its depth. Each drag is one
undo step. The **Depth** row in **Properties › Frame** sets the active layer's depth too.

**Animating depth.** Click the diamond beside a depth to key it on the current frame. The depth
then tweens evenly between keys and holds before the first and after the last. Click a filled
diamond to remove that key.

### Attaching a layer to the camera

Titles, captions and HUDs should stay put while the camera moves. Click the camera icon on a
layer's row in the timeline (or **Attach** in the Layer Depth panel) to attach the layer to the
camera: it then stays exactly where you drew it on screen, whatever the camera does. Draw it as
it should look in the frame, with the camera at rest.

### Preview Through Camera

Choose **View › Preview Through Camera** (also in the Camera layer's right-click menu and in
**Properties › Camera**) to see the stage just as the movie will look. While you preview, the
**Selection** tool still works: click artwork where you see it to select it, and drag it to move
it. It follows the pointer at any depth and zoom. The drawing tools pause; turn the preview off to
draw.

While you're editing, the stage otherwise stays in stage space, so your artwork doesn't jump
around as the camera moves. Test Movie and every export always see through the camera.

### Turning the camera off or removing it

* Hide the Camera layer (its eye) to turn the camera off: the movie then shows the stage as it
  is.
* To remove the camera, click the camera button in the timeline header again, or choose
  **Modify › Timeline › Remove Camera**. If the camera is animated, AniCraft asks first. Undo
  brings it back.

Not there yet: tools other than Selection while previewing, depth inside symbols, depth keys that
move with inserted frames, and cameras or layer depth from Animate files.

## Bones and inverse kinematics

Bones join symbol instances into a jointed **armature**, such as an arm, a leg or a tail. You
then pose the whole chain by dragging its end, and the joints above it turn to follow. This is
called inverse kinematics.

### Building an armature

1. Make each body part a symbol instance (select it and press **F8**).
2. Pick the **Bone** tool (**M**).
3. Drag from one instance (for example, the upper arm) to the next (the forearm). A bone joins
   them, and both move to a new **armature** layer, shown with a bone icon in the timeline.
4. To continue the chain, drag from the last instance (the forearm) to the next one (the
   hand). The new bone always starts at the end of the bone before it, wherever on the
   instance you began, so the chain stays joined.
5. To branch, drag from any instance of the armature to another one. Branches from the first
   instance (the root, such as a torso) start where you begin the drag; branches from any other
   instance start at its joint.

Bones show on the stage as wedges, from a round joint to their end, in the armature layer's
colour. Click a bone to select it.

### Posing

With the **Selection** tool (**V**):

* drag a bone or an instance, and the bones above it turn so the point you grabbed follows the
  pointer (drag a bone's end to pull the end). If the point is out of reach, the chain stretches
  straight towards it;
* drag the root instance, or the joint its first bone starts at, to move the whole armature.

Each drag is one undo step.

### Animating poses

Move the playhead to another frame and pose the armature: AniCraft keys the pose there. Poses
in between are tweened with the span's easing, and each pose is exact on its own frame. Test
Movie and every export show the poses.

### Bone properties

Select a bone to see **Properties › Bone**:

* **Angle**: the joint's turn at the playhead. Changing it keys the pose.
* **Constrain rotation**, with **Limits** (min and max): how far the joint may turn, in every
  pose.
* **Pin**: the bone, and the bones it hangs from, stay put while you drag the bones below it.
  Handy for keeping a shoulder in place while you pose a hand.
* **X translation** and **Y translation**: **Enable** lets the joint slide along the x or y axis
  of what the bone starts on (as that part is turned), so the bone can stretch away from it,
  like a piston or a neck that extends. **Constrain** limits how far, with **Limits** (min and
  max, in pixels). **Slide** sets how far the joint has slid at the playhead, and keys the pose.
  When you drag with the **Selection** tool, joints turn first and slide only to reach where
  turning can't.
* **Spring** and **Damping** (0 to 100): secondary motion. A bone with a spring lags behind
  when the bones it hangs from move, swings past its pose and settles back, like a tail, an
  antenna or a ponytail. A higher **Spring** is stiffer: it swings faster and lags less; 0 is
  no spring. **Damping** sets how quickly the swing dies down: 0 swings on for a while, 100
  absorbs it all.
* **Springs: Enable** turns every spring of the armature on or off. Turn them off while you
  pose, to see exactly the poses you key; Test Movie and every export play them while they
  are on.
* **Remove Bone** (or **Delete**): removes the bone and those below it. Their instances stay
  where they are, on a layer of their own. If the armature has more than one pose, AniCraft asks
  first.

Selecting a bone also selects the instance it moves. Properties then shows that instance's
**Colour effect**, and for movie clips and buttons its **Blending** and **Filters**. Change
them there, and the instance keeps the look in every pose, while its bone still places it.

Springs never change your poses: AniCraft works out the swing from how the armature moves up
to each frame, so jumping to a frame shows exactly what playing to it shows. Each swing fades
out within six seconds of the motion that caused it.

### Bones inside a shape

A shape can have bones of its own, so the shape itself bends, like a tail, a tentacle or a
strand of hair, with no symbols needed.

1. Draw the shape. A shape with points along its length bends more smoothly.
2. With the **Bone** tool (**M**), drag from inside the shape to where the first bone ends.
   The shape moves to a new armature layer. It becomes a drawing object, so shapes you draw
   later don't cut into it.
3. Drag from the end of the last bone to add the next one, or from inside the shape to start
   a branch.

Every point of the outline follows the bones nearest to it. Points near a joint are shared by
the bones that meet there, so the shape bends smoothly. Pose and animate it as you would any
armature: drag its bones with the **Selection** tool, and drag the shape to move it. Lines keep
their width as the shape bends.

**The Bind tool** (**Shift+M**) changes which points follow which bone. With it, the shape's
points show as small squares, or as triangles where several bones share a point. Click a bone
to pick it, and the points it moves fill in. Then:

* click a point (or drag a box around several) to make it follow only that bone;
* **Shift**-click to share the bone with the bones the point already follows;
* **Ctrl**-click (**⌘**-click on a Mac) to take the point away from the bone. A point that no
  bone moves stays where it was drawn.

**Properties › Bone** shows how many of the shape's points the bone moves. **Rebind All** goes
back to the automatic binding. Adding or removing bones rebinds the points you haven't bound
by hand.

**Painting weights.** For a softer or more exact bend than clicks give, paint how much a bone
moves each point. Pick a bone with the **Bind** tool, then turn on **Paint Weights** in
**Properties › Bone**. The outline and its points turn into a heat map of that bone's share:
blue where it moves nothing, through green and yellow, to red where it moves the point alone.
Drag over the shape to paint:

* **Add** gives the bone more of the points under the brush, **Subtract** gives it less, and
  **Smooth** evens out its share, softening a hard edge between two bones;
* **Size** is the brush's radius (**[** and **]** change it), and **Strength** is how much one
  pass changes the weights at its centre. The effect fades to nothing at the rim.

A point's shares always add up to 100 %. What one bone gains the point's other bones lose, in
proportion. A point taken from its only bone goes to the next nearest bone. Each stroke is one
step to undo. Painted points count as bound by hand, so adding a bone later leaves them alone.

**Typing a weight.** Properties › Bone also shows one point's weights as numbers. Click a
point with the Bind tool, or type its number in **Point**. Then type the bone's share in
**Weight**, and **Follows** lists every bone the point follows. Keyboard users can do all of
this without the stage.

**Reshaping a shape that has bones.** At the rest pose (every bone as drawn), use the
**Subselection** tool to edit the shape's outline. You can move points and handles,
double-click the outline to add a point, delete points, or make them smooth or corners. The
bones keep their binding. Moved points keep their weights, and a new point takes the weights
of the points on either side, blended by where it sits between them. A deleted point's
weights go with it. Every pose bends the new outline. At a posed frame AniCraft asks you to go
to the rest pose first.

Bones inside a shape take joint translation and springs too.

Not there yet: symbols and shapes in one armature, reshaping at a posed frame, armatures
that scripts pose while the movie plays, and armatures from Animate files.

## Asset Warp

Asset Warp bends a shape or a bitmap with pins, without bones: a flag in the wind, a squashed
ball, a character's sleeve, a photo curled at one corner.

1. Pick the **Asset Warp** tool (**Shift+Q**, next to Free Transform).
2. Click the artwork to pin it there. The first pin covers it with a **warp mesh**, a fine grid
   that shows faintly while the tool is in hand. Add a few more pins: where the artwork should
   hold still, and where you want to pull it.
3. Drag a pin. The artwork bends around the other pins as you drag, keeping its parts as
   rigid as it can (no shearing or squashing near the pins). With only one pin, the whole thing
   moves. Dragging artwork where there is no pin adds one there and drags it. Each drag is one
   undo step.

Click a pin to pick it (it turns blue), then press **Delete** to remove it, or **Esc** to let
it go. Pins don't move anything when you add them, and the artwork with all its pins where you
placed them looks exactly as you drew it.

**Animating a warp.** Pins live on keyframes, like a pose. Make a keyframe (**F6**), move the
pins there, and add a classic tween between the keyframes: the pins tween with the span's
easing and the artwork bends smoothly between the two shapes. Moving a pin in the middle of a
tween adds a keyframe there. A pin you add later appears on every keyframe of the artwork.

**Properties › Asset Warp** shows the mesh's **resolution** (cells along the artwork's longer
side: more cells bend more smoothly, fewer are faster), how many pins it has, **Remove Pin**
and **Remove Warp** (the artwork goes back to how you drew it, on every keyframe). The same
commands are in **Modify › Asset Warp**.

The warped artwork becomes a drawing object, so shapes you draw later don't cut into it. If you
reshape it with another tool afterwards, the warp starts again from the new shape, with its
pins where they are. A gradient fill keeps its place while the outline bends; a bitmap bends
with the mesh. Every export shows the bent artwork. SVG writes a bent bitmap as an image, and
HTML and Lottie can't bend a bitmap yet (the export report says so).

Not there yet: warping symbols and text, tying pins to bones, and warped artwork from Animate
files.

## Next steps

* More on layers and keyframes: [The timeline](timeline.md).
* Tween kinds, easing and the Motion Editor: [Tweens](tweens.md).
