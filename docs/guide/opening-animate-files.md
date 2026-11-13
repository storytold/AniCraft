# Opening Animate files

AniCraft opens documents made in Adobe Animate, both `.fla` files and XFL folders, and brings
across as much as it can as artwork you can keep editing. Anything it can't bring across yet is
listed for you, never silently dropped.

AniCraft is not made by or affiliated with Adobe; it reads these files from their public
structure and from observation. On a Mac, read **⌘** wherever this chapter says **Ctrl** (so
**Ctrl+O** is **⌘O**), and **⌥** for **Alt**.

## Opening a file

1. Choose **File › Open…** (**Ctrl+O**, **⌘O** on macOS).
2. Pick a `.fla` file, or the `.xfl` file inside an XFL folder.
3. AniCraft opens it as a new, untitled document, and the status bar says what came across and
   what didn't.

Other ways in:

* **Drop** a `.fla`, an `.xfl` file or an XFL folder onto AniCraft's window.
* **File › Open Recent** reopens Animate files you opened before.
* On **macOS**, double-click a `.fla` or `.xfl` in Finder, drop it on AniCraft's Dock icon, or
  use Open With › AniCraft.
* On **Windows** and **Linux**, the installers list AniCraft under "Open with" for FLA and XFL
  files, without changing your default app.

If the document you have open has unsaved changes, AniCraft asks first.

In the web version, open the `.fla`: a browser can hand over files but not folders, so an XFL
folder can't be opened there.

## Saving: your Animate file is never touched

An opened Animate file comes in **untitled**. When you save, AniCraft asks for a new name and
writes an `.anicraft` document, so the original `.fla` or XFL is never overwritten. The status
bar reminds you: "Save to keep it as an AniCraft document."

AniCraft doesn't save back to `.fla` or XFL.

## What comes across

| Area | What you get |
|---|---|
| Document | Stage size, frame rate and colour; every scene, in order |
| Layers | Names, colours, visibility, lock, outline, height; guide, mask and folder layers, masks inside folders and symbols too |
| Frames | Keyframes and their lengths, frame labels |
| Tweens | Classic tweens with their ease (custom eases of any number of points exactly, and eases set per property: position, rotation, scale, colour, filters), shape tweens with shape hints, motion guides and Orient to path |
| Drawing | Shapes as merged drawings, drawing objects and groups, rectangle and oval primitives, solid and gradient fills, strokes with caps, joins, miter limit, dashed and dotted styles |
| Symbols | Graphics, movie clips and buttons with their own timelines, nested to any depth; Library folders; loop mode and first frame; 9-slice guides |
| Instances | Position and transform, instance names, colour effects in every style (Brightness, Tint, Advanced, Alpha), blend modes, all seven filters |
| Bitmaps | Library bitmaps, placed bitmaps and bitmap fills, with Allow Smoothing, when the image is stored as PNG or JPEG |
| Sounds | Sounds stored as WAV, Ogg Vorbis or MP3, in their Library folders, with waveforms on the timeline; on the keyframes that use them (button states included) with their sync (Event, Start, Stop, Stream), repeat or loop, and effect: Left or Right Channel, the fades, or a custom envelope |
| Videos | A video in the Screen Video format comes in as a still of its first frame, placed where the video was and sized to it; every other video is named in the report with its format and length (AniCraft doesn't play video) |
| Text | Static text as editable text with its font, size, colour, bold and italic, alignment, spacing, width and wrapping; dynamic and input text as fields, with their names, boxes, borders and line types |
| Scripts | Frame scripts in scenes and symbols, kept exactly as written, with symbol linkage classes and the document class |

Raw shapes open as raw shapes, joined into one drawing per keyframe as they were in Animate, so
the Selection tool, Subselection and the Paint Bucket work on them straight away. Drawing
objects and groups stay objects.

## Scripts

Every frame script comes across as it was written, in the language the file's publish settings
name (usually ActionScript 3.0). Open them in the Actions panel's Script view (**F9**).

* Scripts in the ActionScript 3.0 subset AniCraft runs (timeline control, buttons, variables,
  `if` and loops, functions, keyboard and enter-frame listeners, text fields) **play in Test
  Movie** and the HTML canvas export.
* The rest are **kept, not run**: they stay with your document, unchanged, and the Script view
  shows what keeps each one from running.
* Scripts attached to instances in older documents (`on (release) { … }`) are left out and
  reported.

The status bar's report says how many scripts run and how many are only kept. See
[Interactivity](interactivity.md#frame-scripts-in-actionscript-30) for what the subset covers.

## The import report

After opening, the status bar sums up the import: "Imported everything in the file", or
"Imported." followed by what runs, what is kept, what isn't supported yet (with a count for
each) and any fonts that are missing. Point at the status bar to read the whole report.

Sounds and videos that don't come across are **named**, with the reason, for example
"2 sounds in formats AniCraft can't decode (theme.aif: AIFF; voice.wav: WAV in IMA ADPCM)" or
"1 videos AniCraft can't play (intro.flv: FLV, On2 VP6, 4.5 s)". The first few are shown, then
how many more.

### Drawn the nearest way AniCraft can

These come across, but look a little different from Animate, and the report counts them:

* gradient and bitmap strokes (drawn in one colour);
* ragged, stipple and hatched strokes (drawn solid);
* reverse loop modes (played forward) and extra spins on classic tweens (turned the shortest
  way);
* in text: justified text (drawn left-aligned), first-line indents, superscript and subscript,
  vertical text, text rendered as HTML, and lines that mix styles (drawn in their main style).
  A field whose lines use different styles becomes one text block per style, stacked as in
  Animate.

### Left out, for now

* **Motion tweens** come across as keyframes, without their property curves or motion path.
* **Bitmaps and sounds stored only in Animate's own internal format.** Animate keeps some media
  in an undocumented form; those are listed, by name for sounds, instead of guessed at. Media
  kept as PNG, JPEG, WAV, Ogg or MP3 comes across.
* Sounds in formats AniCraft doesn't decode (AIFF, compressed WAV such as ADPCM, FLAC, AAC);
  convert them to WAV, Ogg or MP3 and import them again (**File › Import › Import to
  Library**).
* Sound in and out points (the sounds play whole), and sound effect names AniCraft doesn't know
  (played without an effect).
* **Video playback.** Videos other than Screen Video (Sorenson H.263, On2 VP6, H.264, linked
  MPEG-4 files) are named in the report, not shown; their instances are left out.
* Frame comments, filters on text, and linkage classes on bitmaps and sounds.
* Camera layers and bone armatures don't come across as a camera or an armature.
* Anything AniCraft doesn't recognise at all is counted as "other elements".

## Fonts

Text is laid out with the fonts on your computer. If a font the document uses isn't there,
AniCraft draws that text in a fallback font and names the missing fonts in the report, for
example "Fonts not on this computer, drawn in Arial: Futura." Install the font and open the file
again to see the text as designed.

Once you save, the `.anicraft` document keeps the letter shapes, so it looks the same on
computers without the font.

## Old, damaged and unusual files

* A `.fla` saved by **Flash CS4 or earlier** uses an older format AniCraft can't read. AniCraft
  says so: open it in Animate and save it again, then open that copy.
* A renamed `.fla` still opens; AniCraft recognises it by what is inside.
* Archives made by different zip tools open, large ones included: a zipped XFL folder (the
  document inside one top folder, with or without the `__MACOSX` folder macOS adds), and a
  self-extracting archive renamed to `.fla`. A zip holding several documents in their own
  folders says which ones it holds: unzip it and open the one you want.
* A **damaged or suspicious** file fails with one sentence that names the problem, and never
  crashes AniCraft. Nothing from it is ever unpacked onto your disk.
* When only **part** of a file is damaged (one bitmap, one sound or one library symbol), the rest
  opens and the report counts what was left out, such as "1 bitmaps whose files couldn't be
  read".
* A document that opens without a stage colour gets Animate's default, white.
* A document you double-click, or open with **Open With**, that can't be opened says why in the
  status bar.

## Tips

* **Save straight away** as an `.anicraft` document, then read the report while the original is
  still beside you to compare.
* **Install the fonts** the document uses before opening it, so text lays out as designed.
* **Re-save in Animate as XFL** when you can: an XFL folder keeps the original image and sound
  files, so more of them come across.
* **Try Test Movie** (**Ctrl+Enter**) to see which scripts play; Output lists the ones that
  don't and why.
* Check motion tweens: they arrive as keyframes, so you may want to tween them again in
  AniCraft. See [Tweens](tweens.md).
* Coming from Animate in general? [Coming from Animate](coming-from-animate.md) maps panels,
  tools and shortcuts.
