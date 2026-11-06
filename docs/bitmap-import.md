# Bitmap import

Choose **File › Import › Import Bitmap…** (`Ctrl+R`, `⌘R` on macOS), or use the command palette.
Select a still PNG or JPEG. Decoding runs in the background, then adds a Library asset and
places it at the stage centre on the active layer/playhead. Import is one undo step. Opening
or creating another document cancels a pending import.

Images start at their original pixel size. Selection, Free Transform, align, duplicate,
alpha, tint and classic tweens use the normal instance commands. Double-click a Library bitmap
or click **Place on stage** to add another instance. Pixels are shared. **Break Apart** retains
a bitmap-filled shape whose path can be edited to clip the image.

## Files and limits

PNG supports indexed colour, transparency and 16-bit input (converted to RGBA8). JPEG import
applies EXIF orientation. Colours are treated as sRGB; ICC conversion is not supported yet.
Animated PNG and other image formats are rejected with an explanation.

Saving embeds images in `.anicraft` v3, so the source file can be moved or deleted. Repeated
instances share one embedded PNG. Existing v1/v2 documents open; older releases cannot open
newly saved v3 files. Stage and PNG/GIF/SVG exports include images. SVG embeds PNG data with
transforms, alpha, tint and clipping. Bitmap mask artwork follows existing fill-area semantics:
its rectangle masks, without using image alpha.

Limits: 80 MiB input; 16,777,216 pixels and 16,384 px per image side; 256 MiB decoded pixels and
4096 distinct images per document. Reduce source dimensions when a limit is exceeded.

## Commands

`file.import_bitmap` accepts:

```json
{"path":"background.png","name":"Background","place":true,"x":640,"y":360}
```

Only `path` is required. Name defaults to the filename. `place` defaults to `true`; use `false`
for Library-only import, including on locked layers. `x`/`y` specify the image centre in stage
pixels and default to the stage centre. Results include `symbol`, optional placed `element`,
`name`, `width` and `height`. Available through CLI `exec`, scripts and JSON-lines `serve`:

```sh
anicraft-cli exec shot.anicraft file.import_bitmap '{"path":"background.png"}' --save shot.anicraft
```

XFL/FLA imports include bitmap instances backed by standard PNG/JPEG package assets. Missing
media, proprietary bitmap payloads and XFL bitmap fills remain in the import report. Image
sequences, relinking and pixel editing remain future work.
