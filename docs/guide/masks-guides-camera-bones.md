# Masks, guides, camera and bones

Some layers do a special job. This chapter covers mask layers, guide layers and motion guides,
the camera layer, and bones for posing a character.

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
**Zoom** (5 to 2,000 %), **Rotation**, and a **Tint** over the whole view with its amount.
**Reset** puts the camera back to the plain stage: centred, 100 %, no turn and no tint.

### Animating the camera

The camera animates like any object. Add keyframes on the Camera layer, change the camera on
each, and create a classic or motion tween between them. In the Motion Editor, the camera's zoom
is **Scale X**.

The Camera layer holds only the camera. Draw your artwork on other layers.

### Depth and parallax

Once the scene has a camera, each layer can have a **Depth** in **Properties › Frame**. A layer
with more depth sits farther behind the stage and moves less as the camera pans, which gives a
sense of distance (parallax). A negative depth puts the layer in front.

### Preview Through Camera

Choose **View › Preview Through Camera** (also in the Camera layer's right-click menu and in
**Properties › Camera**) to see the stage just as the movie will look. While you preview, the
drawing tools pause; turn the preview off to edit artwork again.

While you're editing, the stage otherwise stays in stage space, so your artwork doesn't jump
around as the camera moves. Test Movie and every export always see through the camera.

### Turning the camera off or removing it

* Hide the Camera layer (its eye) to turn the camera off: the movie then shows the stage as it
  is.
* To remove the camera, click the camera button in the timeline header again, or choose
  **Modify › Timeline › Remove Camera**. If the camera is animated, AniCraft asks first. Undo
  brings it back.

Not there yet: editing artwork while previewing through the camera, a perspective mode where
depth scales layers, depth inside symbols, and cameras from Animate files.

## Bones and inverse kinematics

Bones join symbol instances into a jointed **armature**, such as an arm, a leg or a tail. You
then pose the whole chain by dragging its end, and the joints above it turn to follow. This is
called inverse kinematics.

### Building an armature

1. Make each body part a symbol instance (select it and press **F8**).
2. Pick the **Bone** tool (**M**).
3. Drag from one instance (for example, the upper arm) to the next (the forearm). A bone joins
   them, and both move to a new **armature** layer, shown with a bone icon in the timeline.
4. To continue the chain, drag from the end of the last bone to the next instance (the hand).
5. To branch, drag from any instance of the armature to another one.

Bones show on the stage as wedges, from a round joint to their end, in the armature layer's
colour. Click a bone to select it.

### Posing

With the **Selection** tool (**V**):

* drag a bone or an instance, and the bones above it turn so it follows the pointer. If the point
  is out of reach, the chain stretches straight towards it;
* drag the root instance to move the whole armature.

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
* **Remove Bone** (or **Delete**): removes the bone and those below it. Their instances stay
  where they are, on a layer of their own. If the armature has more than one pose, AniCraft asks
  first.

Not there yet: bones inside shapes (shape armatures), springs, joints that slide, armatures
that scripts pose while the movie plays, and armatures from Animate files.

## Next steps

* More on layers and keyframes: [The timeline](timeline.md).
* Tween kinds, easing and the Motion Editor: [Tweens](tweens.md).
