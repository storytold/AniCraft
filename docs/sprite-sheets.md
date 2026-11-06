# Sprite sheet export

Choose **File › Export › Export Sprite Sheet + JSON…**, or find it in the command palette.
Choose a PNG name, frame range, scale, columns and background, then click **Export**. The
dialog previews the output size. Columns `0` means automatic in the dialog. The initial scale
fits the whole timeline where possible; it is displayed and can be changed. Export uses a
snapshot of the document, so subsequent edits do not change the running export.

The output is a fixed grid: every frame has the same dimensions, with no trimming, padding or
rotation. Frames fill left to right, then top to bottom. Transparent background is on by
default; turn it off to include the document's stage background. Unused cells stay transparent.
Sheets are limited to 8192 px per side, 16,777,216 total pixels and 10,000 frames. Lower scale
or shorten the range if the export exceeds these limits.

The PNG and JSON use the same basename, e.g. `walk.png` and `walk.json`. Existing files ask for
replacement confirmation, including when only the JSON exists. Both files are staged before
replacement. Individual replacements are atomic; an OS interruption between replacements can
leave a mismatched pair, which can be repaired by exporting again.

## CLI and commands

```sh
cargo run -p anicraft-cli -- sprite-sheet --sample --out /tmp/walk.png --from 1 --to 12 --scale 0.25 --columns 4
```

CLI frame numbers are **1-based, inclusive**. Omit `--columns` for automatic placement.
`--transparent false` includes the stage background. `--overwrite yes` authorizes replacement
of the PNG and JSON.

The same export is available through `exec`, scripts and JSON-lines `serve` as
`file.export_sprite_sheet`:

```json
{"path":"walk.png","from":0,"to":11,"scale":0.25,"columns":4,"transparent":true}
```

Engine frame numbers are **0-based, inclusive**. Omitted range endpoints select the beginning
and end of the timeline; scale defaults to `1`. Optional columns must be between `1` and the
number of exported frames. Add `"confirm": true` to authorize replacing existing outputs.
The command returns both output paths, frame count, sheet dimensions, columns and rows.

## JSON v1

This is AniCraft metadata, not a game-engine-specific atlas format. Coordinates are integer
pixels from the top left of the PNG. `image` is the PNG filename relative to the JSON's folder.
`frame` is the **0-based source timeline frame**, not the cell's index. Array order is playback
order. `duration_ms` is a floating-point duration with no integer-millisecond rounding;
`fps` retains the source document's frame rate.

```json
{
  "format": "anicraft-sprite-sheet",
  "version": 1,
  "image": "walk.png",
  "size": {"w": 128, "h": 64},
  "columns": 2,
  "rows": 1,
  "fps": 24.0,
  "frames": [
    {"frame": 0, "rect": {"x": 0, "y": 0, "w": 64, "h": 64}, "duration_ms": 41.666666666666664},
    {"frame": 1, "rect": {"x": 64, "y": 0, "w": 64, "h": 64}, "duration_ms": 41.666666666666664}
  ]
}
```
