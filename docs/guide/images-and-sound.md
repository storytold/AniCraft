# Images and sound

This chapter covers bringing pictures, SVG drawings and sounds into AniCraft, and putting sound
to work on the timeline: sync, loops, fades, volume and sounds inside symbols and buttons.

On a Mac, read **⌘** wherever this chapter says **Ctrl**, and **⌥** for **Alt**.

## What AniCraft can bring in

| Kind | Files | Comes in as |
|---|---|---|
| Images | PNG, JPEG | A bitmap in the Library, placed on the stage as a bitmap |
| Drawings | SVG | Editable shapes, groups, gradients, masks and text |
| Sounds | WAV, Ogg Vorbis, MP3 | A sound in the Library, played from a keyframe |

Anything else is turned away with one line in the status bar that says what AniCraft takes.

## Importing images

### Onto the stage

1. Click the frame where the image should appear.
2. Choose **File › Import › Import to Stage…** (**Ctrl+R**, **⌘R** on macOS) and pick a PNG
   or JPEG.
3. The image arrives on a new layer named after the file, centred on the stage and selected.

It goes into the Library at the same time. From here it is ordinary artwork: move, scale,
rotate, fade, tween and mask it like anything else. It stays smooth at every size, on the stage
and in every export.

Images are stored inside your `.anicraft` document, so you can move the document to another
computer without the original files. JPEGs are converted to PNG without losing quality, which
can make the document larger than the JPEG was.

### Into the Library only

Choose **File › Import › Import to Library…** to add an image without placing it. Then, in the
Library:

* drag it onto the stage, where it lands under the pointer on the active layer at the playhead;
* or select it and press **Place on Stage** (or **Enter**, or right-click › Place on Stage) to
  put it at the centre of the view;
* or double-click it to add another copy centred on the stage.

Every copy shares the one image, so placing it many times costs nothing extra.

### Smoothing

Select a bitmap in the Library to see its properties. Turn off **Allow Smoothing** for pixel
art: the bitmap then keeps hard pixel edges on the stage, in renders, in SVG and in the HTML
page.

### Swap Bitmap

To show a different image in the same place, select the placed bitmap and use **Swap…** on the
**Bitmap** row of **Properties**. Pick another bitmap from the list. A placed bitmap keeps its
position and transform and takes the new image's size; a shape painted with a bitmap fill keeps
its outline and shows the new image inside it. With several bitmaps selected, they all change.

## Importing SVG drawings

SVG files from illustration tools come in as artwork you can keep editing, not as a flat picture.

1. Click the frame where the drawing should start.
2. Choose **File › Import › Import to Stage…** (**Ctrl+R**) and pick the `.svg` file.
3. The drawing arrives on a new layer named after the file, centred and selected, as one undo
   step.

### What comes across

* Paths and basic shapes, with their fills and strokes, including stroke caps, joins, miter
  limits and dashes.
* Groups, reused parts (`use` and symbols), transforms and the drawing's view box.
* Inline and attribute styles, and opacity.
* Linear and radial gradients, with their spread (reflect, repeat), linear-RGB blending, focal
  point and transform.
* **Clip paths and masks.** A drawing with clipping comes in as a layer folder named after the
  file. Each clipped group sits on a masked layer under a mask layer that holds the clip's shape,
  so you can move or reshape the mask afterwards. Clips that cut nothing, like the frame clip
  many design tools add, are dropped. Solid masks become ordinary (Shape) masks; faded, grey or
  gradient masks become **soft masks** (mask type Luminance, or Alpha when the SVG says so).
* **Text** comes in as text you can edit, one block per positioned line, with its font, size,
  weight, italic, colour, letter spacing and alignment. Text on a path comes in as text on a
  path. Characters the SVG places one by one (moved, shifted or turned) come in as small blocks
  of their own where they are. Fonts that aren't on your computer fall
  back to the default, and the status bar names them.

### What doesn't come across yet

AniCraft lists these in the status bar after the import, instead of guessing:

* images embedded in the SVG, filters, style sheets (inline styles are read) and pattern fills;
* gradient strokes (drawn in one colour).

### An SVG as a symbol

**File › Import › Import to Library…** with an SVG makes one **graphic symbol**, named after
the file, that holds the drawing's layers. Place it as often as you like; edit the symbol and
every copy changes. See [Symbols and the Library](symbols-and-library.md).

## Dropping files from your desktop

You can drag files straight from your file manager onto AniCraft's window. Each drop is one
undo step, however many files it brings, and a highlight shows where they will land.

| Drop it on | What happens |
|---|---|
| The **stage** | Drawings and images go onto the active layer at the playhead, centred where you let go. A sound goes onto the active layer's keyframe. |
| A layer's **frame** in the timeline | A sound goes onto that keyframe. An image or drawing goes onto that layer at that frame, in a new keyframe if there isn't one. |
| The **Library** | The files go into the Library folder under the pointer. An SVG becomes a graphic symbol. |

A drawing whose clipping needs several layers can't sit on one layer, so dropped on the stage
it arrives as a graphic symbol instead. Locked layers, folders and the camera layer say why
they won't take a drop.

On the desktop, AniCraft learns exactly where you dropped once you move the pointer again; if
you don't, the drop lands in the middle of the view. Dropping `.anicraft`, `.fla` or `.xfl`
files opens them instead (see [Opening Animate files](opening-animate-files.md)).

## Importing sounds

AniCraft reads WAV, Ogg Vorbis and MP3 files.

* **File › Import › Import to Stage…** (**Ctrl+R**) with a sound puts it on a new layer, named
  after the file, starting at the playhead. The layer's frames run as long as the sound, so you
  see its whole waveform.
* **File › Import › Import to Library…** only adds it to the Library.

Sounds are saved inside the document exactly as imported.

In the Library, select a sound to see its waveform with a play button. Drag it onto the stage to
put it on the active layer's keyframe, or use **Place on Timeline**. Deleting a sound that
keyframes use asks first, and you can undo it.

## Sounds on keyframes

As in Animate, a sound belongs to a keyframe. Select the keyframe and open the **Sound** rows
in **Properties**.

1. **Sound**: pick one from the Library, or **None** to take it off.
2. **Sync**: how it plays (see below).
3. **Repeat**: play it a set number of times in a row (**Repeat** with a count), or choose
   **Loop** to play it again and again.
4. **Effect**: a fade or pan (see below).

Below the rows you see the sound's length, sample rate, channels and format.

### Sync

| Sync | What it does |
|---|---|
| **Event** | Plays in full from this keyframe, independent of the timeline. Good for clicks and hits. |
| **Start** | Like Event, but doesn't start again if the sound is already playing. |
| **Stop** | Stops this sound. |
| **Stream** | Tied to the frames: plays over this span and follows the playhead. Use it for dialogue and lip sync. |

A looping Event sound plays to the end of the movie; a looping Stream sound to the end of its
span.

### Waveforms on the timeline

The timeline draws each sound's waveform over its frames, so you can line up keys with beats
and syllables. The waveform follows the sound's effect, so a fade-out shows as a shrinking
wave.

### Effects and the envelope

The **Effect** menu has Animate's presets: **None**, **Left channel**, **Right channel**,
**Fade left to right**, **Fade right to left**, **Fade in** and **Fade out**. Each is shaped
over the time the sound is actually heard.

For your own volume curve, press **Edit…** next to Effect (or choose **Custom**). The **Edit
Envelope** window shows each channel's waveform with its volume line:

1. Click a channel to add a volume point (up to 16).
2. Drag a point to move it, or right-click it to delete it.
3. Or select a point and type its time and its left and right levels.
4. Press **Play** to hear the sound with its envelope.

Times can read in frames or seconds, and you can zoom in, out or fit the whole sound. Editing a
preset turns it into **Custom**.

### Layer volume and mute

A layer that holds sounds has a **Volume** (0–100 %) and **Mute** in **Properties**, and a
speaker toggle in its timeline row. Volume applies to the layer's sounds and to sounds inside
the symbols on it. A muted layer is silent while you work and in every export, which is handy
while you time other tracks.

## Sounds inside symbols and buttons

Sounds placed on a symbol's own timeline play too:

* in a **graphic**, they play on the symbol's frames as the parent timeline shows them; looping
  and the first frame setting apply, and a stream reached part-way through is heard from there;
* in a **movie clip**, they play on the clip's own loop from when it appears;
* volumes multiply down through nested symbols.

**Buttons** can have sounds on their state frames. A sound on the **Over** frame plays when the
pointer goes over the button, and one on **Down** when it is pressed. As in published Animate
movies, the **Up** frame's sound plays when the pointer leaves, and the **Hit** frame's when you
let go over it. Button sounds play in Test Movie and the HTML canvas page, and stay out of the
soundtrack: audio and video exports don't include them. See
[Interactivity](interactivity.md#buttons).

## Hearing it: playback and scrubbing

* Press **Enter** (**Control › Play**) to play the movie with its sound. The playhead follows the
  sound, so picture and sound stay together.
* **Stream** sounds pick up wherever the playhead jumps.
* **Drag the playhead** to scrub: you hear short snippets of the sound under it, which helps with
  lip sync.
* **Test Movie** (**Ctrl+Enter**) plays the sounds too, including those in movie clips, graphics
  and buttons.

The web version of AniCraft plays silently for now. Sounds still import, show on the timeline
and export.

## Getting sound out

* **File › Export › Export Audio…** writes the soundtrack as a WAV file.
* **Export Video…** includes the soundtrack.
* The HTML canvas page plays the sounds, with their envelopes and volumes.

Lottie animations carry no sound. See [Exporting and Publish](exporting-and-publish.md).

## Not yet

* Picking a colour from a bitmap's pixels with the Eyedropper.
* Panning beyond what the envelope does.
* Sound playback in the web version.
