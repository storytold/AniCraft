# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with AniCraft.app and the
`Applications` link side by side on a Paper card, an Ink arrow between them, on the AniCraft
yellow. `package.sh` copies these files into the image; nothing here is generated at build time,
so the DMG still builds with `hdiutil makehybrid` (no mounted device, no Finder scripting on CI).

| File | What |
|---|---|
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (16-colour palette, Deflate, sRGB; about 11 KB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, AniCraft.app at (190, 196), `Applications` at (470, 196), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Draws `background.tiff` with Pillow in the icon's three colours (`assets/app-icon/README.md`: Ink `#0b0b0c`, Paper `#f2f1ee`, AniCraft `#dac604`) and writes `dmg-layout.DS_Store` from the layout above. |

## The volume name has no version

The mounted volume is called `AniCraft`, not `AniCraft <version>`. The layout points at the
background through an alias that includes the volume name, and Finder resolves it by that name:
with a versioned name the window keeps its size and icon positions but shows no background. The
DMG file name (`anicraft-<version>-macos-<arch>.dmg`) still carries the version.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a
  background, so the area under both icons stays light (Paper).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and
  of the label strip under it.

## Regenerate

Edit the layout constants or `draw()` in `generate.py`, then run, on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py            # --preview also writes preview.png (don't commit it)
```

Both outputs are byte-for-byte reproducible and carry nothing from the machine that ran it:
`dmg-layout.DS_Store` is written from scratch, its background alias holding only the volume name
and `/.background/background.tiff`. The window is 660 × 432 with Finder's 32 pt title bar; the
content area is 660 × 400.

This follows the Craft family's DMG layout (PhotoCraft's and VectorCraft's `packaging/macos/dmg/`).
