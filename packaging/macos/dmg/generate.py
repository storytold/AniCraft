#!/usr/bin/env python3
"""Regenerate background.tiff and dmg-layout.DS_Store for the macOS DMG window (see README.md here).

Needs:
    pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3

Runs on any OS, no Mac and no fonts needed: the background is drawn here with Pillow from the
icon's palette (assets/app-icon/README.md). Both outputs come from this script alone, so they
carry nothing from the machine that made them: no local paths, user or disk names, dates or
volume UUIDs.

    python3 packaging/macos/dmg/generate.py
"""

import datetime
import os
import sys

from ds_store import DSStore
from mac_alias import Alias, TargetInfo, VolumeInfo
from PIL import Image, ImageCms, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))

# Keep in sync with package.sh (volume name) and README.md (layout).
VOLUME = "AniCraft"
WIDTH, HEIGHT = 660, 400  # window content, pt; the background's 1x size
TITLE_BAR = 32
ICON_SIZE = 128
PALETTE = 16  # colours in background.tiff
ICONS = {"AniCraft.app": (190, 196), "Applications": (470, 196)}
# The icon's three colours.
INK = (0x0B, 0x0B, 0x0C)
PAPER = (0xF2, 0xF1, 0xEE)
FIELD = (0xDA, 0xC6, 0x04)
# A fixed date for the alias and the colour profile. Finder never matches it against the image
# (each build is a new volume): it finds the background by volume name and path.
FIXED_DATE = datetime.datetime(2026, 11, 9, tzinfo=datetime.timezone.utc)
SUPERSAMPLE = 4  # drawn this much larger, then reduced: smooth edges without Pillow's AA paths


def draw(scale):
    """The background at `scale` (1 or 2): the AniCraft field, a Paper card behind both icons and
    their labels (Finder draws the labels in black), and an Ink arrow from the app to Applications.
    Nothing is drawn within 10 pt of an icon box or its label strip."""
    k = scale * SUPERSAMPLE
    im = Image.new("RGB", (WIDTH * k, HEIGHT * k), FIELD)
    d = ImageDraw.Draw(im)

    def box(x0, y0, x1, y1):
        return [round(x0 * k), round(y0 * k), round(x1 * k), round(y1 * k)]

    # Paper card with an Ink edge, like the icon's line work.
    d.rounded_rectangle(box(60, 92, 600, 316), radius=28 * k, fill=PAPER, outline=INK, width=3 * k)
    # A thin Ink rule along the top and bottom of the field, echoing the engraving.
    for y in (36, 364):
        d.rectangle(box(60, y - 1, 600, y + 1), fill=INK)
    # The arrow: shaft and head, between the icon boxes (app box ends at 254, Applications starts at 406).
    cy = ICONS["AniCraft.app"][1]
    d.rounded_rectangle(box(274, cy - 4, 368, cy + 4), radius=4 * k, fill=INK)
    d.polygon([(362 * k, (cy - 18) * k), (392 * k, cy * k), (362 * k, (cy + 18) * k)], fill=INK)
    return im.resize((WIDTH * scale, HEIGHT * scale), Image.Resampling.LANCZOS)


def quantize(im):
    # No dithering: dither noise compresses badly and the artwork has flat fills.
    return im.quantize(PALETTE, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)


def srgb_profile():
    # LittleCMS's built-in sRGB ("No copyright, use freely"), with its creation date fixed.
    icc = bytearray(ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes())
    d = FIXED_DATE
    icc[24:36] = b"".join(v.to_bytes(2, "big") for v in (d.year, d.month, d.day, 0, 0, 0))
    if icc[84:100] != bytes(16):
        sys.exit("error: the sRGB profile has an ID; recompute it after fixing the date")
    return bytes(icc)


def write_tiff(out):
    """1x (72 dpi) and 2x (144 dpi) in one TIFF, as `tiffutil -cathidpicheck` writes them."""
    one, two = (quantize(draw(s)) for s in (1, 2))
    two.encoderinfo = {"dpi": (144, 144)}  # the rest as the first page
    one.save(
        out,
        "TIFF",
        save_all=True,
        append_images=[two],
        compression="tiff_adobe_deflate",  # Pillow's name for TIFF compression 8 (Deflate)
        dpi=(72, 72),
        icc_profile=srgb_profile(),
    )
    with Image.open(out) as im:
        im.seek(1)
        if im.info.get("dpi") != (144, 144):
            sys.exit("error: the 2x page isn't 144 dpi (needs Pillow 12 or later)")
        strips = []
        for page in (0, 1):
            im.seek(page)
            strips += zip(im.tag_v2[273], im.tag_v2[279])  # StripOffsets, StripByteCounts
    # TIFF keeps directories and tag data on even offsets. When a strip ends on an odd one, libtiff
    # leaves the padding byte after it uninitialised, so the file changed from run to run with
    # identical pixels. Zero it: nothing refers to that byte.
    starts = {o for o, _ in strips}
    with open(out, "r+b") as f:
        for o, n in strips:
            if (o + n) % 2 and o + n not in starts:
                f.seek(o + n)
                f.write(bytes(1))


def background_alias():
    # What Finder stores for /.background/background.tiff on the volume, minus anything about the
    # machine. Disk type 5 (ejectable), the volume flags and the folder/file ids (25, 26) are the
    # values Finder writes on such an image; the ids are only hints, as the volume's creation date.
    volume = VolumeInfo(VOLUME, FIXED_DATE, b"H+", 5, 0x0D02, b"\0\0")
    volume.posix_path = f"/Volumes/{VOLUME}"
    target = TargetInfo(0, "background.tiff", 25, 26, FIXED_DATE, b"\0\0\0\0", b"\0\0\0\0")
    target.folder_name = ".background"
    target.cnid_path = [25]
    target.carbon_path = f"{VOLUME}:.background:\0background.tiff"
    target.posix_path = "/.background/background.tiff"
    return Alias(volume=volume, target=target).to_bytes()


def write_ds_store(out):
    if os.path.exists(out):
        os.remove(out)
    with DSStore.open(out, "w+") as d:
        d["."]["bwsp"] = {
            "ShowStatusBar": False,
            "ShowToolbar": False,
            "ShowTabView": False,
            "ContainerShowSidebar": False,
            "ShowSidebar": False,
            "WindowBounds": f"{{{{200, 528}}, {{{WIDTH}, {HEIGHT + TITLE_BAR}}}}}",
        }
        d["."]["icvp"] = {
            "viewOptionsVersion": 1,
            "backgroundType": 2,
            "backgroundImageAlias": background_alias(),
            "backgroundColorRed": 1.0,
            "backgroundColorGreen": 1.0,
            "backgroundColorBlue": 1.0,
            "gridOffsetX": 0.0,
            "gridOffsetY": 0.0,
            "gridSpacing": 100.0,
            "arrangeBy": "none",
            "showIconPreview": True,
            "showItemInfo": False,
            "labelOnBottom": True,
            "textSize": 16.0,
            "iconSize": float(ICON_SIZE),
        }
        d["."]["vSrn"] = ("long", 1)
        for name, pos in ICONS.items():
            d[name]["Iloc"] = pos


def main():
    write_tiff(os.path.join(HERE, "background.tiff"))
    write_ds_store(os.path.join(HERE, "dmg-layout.DS_Store"))
    if "--preview" in sys.argv:
        draw(1).save(os.path.join(HERE, "preview.png"))


if __name__ == "__main__":
    main()
