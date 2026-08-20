#!/usr/bin/env python3
"""Regenerate the Tool-Kit app icon set from mark.png.

    python3 -m venv /tmp/iconvenv && /tmp/iconvenv/bin/pip install pillow
    /tmp/iconvenv/bin/python apps/desktop/src-tauri/icons/make-icons.py

`mark.png` is the two-tone source artwork: a 512px transparent PNG holding
nothing but the rings. It is the closest thing to a vector master this project
has, so it is kept alongside the generated files rather than thrown away.

The mark ships in two flat colours with no blended pixels between them, so the
recolour below is a per-family RGB swap that leaves every alpha byte alone. The
pattern is reproduced exactly; only hue moves.

Colours are the warm ramp from `src/ui/tokens.css`. Every step carries a yellow
cast, which is the palette's defining property: a cool grey beside the app's own
surfaces reads as broken, so "slate" here means warm graphite, not blue-grey.

The plate is what the colour change costs. Bright cyan and amber separated the
mark from any background on their own; graphite rings cannot, and a transparent
version loses whichever ring matches the wallpaper. The plate takes over that
job, and the hairline rim keeps its edge visible on a dark desktop, the one
place a charcoal plate would otherwise dissolve.

Tray icons are NOT generated here. They are template images built from the
mark's alpha, not this file's, and they are unaffected by a recolour.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

HERE = Path(__file__).resolve().parent
MARK = HERE / "mark.png"
PUBLIC = HERE.parents[1] / "public" / "icon.png"

# --- Palette: src/ui/tokens.css --------------------------------------------
PLATE = (0x1D, 0x1B, 0x16)  # warm-900, the app window's own --surface-solid
RIM = (0x4A, 0x49, 0x45)  # warm-600
RING_A = (0xEC, 0xEB, 0xE5)  # warm-100, = --ink
RING_B = (0x77, 0x76, 0x73)  # warm-500, = --ink-3

# macOS draws every app icon body at 0.809 of the canvas (Apple's 824/1024
# grid). Measured off Notes, Music, and Preview, which all agree exactly.
# Diverging from it makes the icon look oversized in a Dock row.
BODY = 0.809
INSET = 0.76  # mark width as a share of the plate
RIM_W = 0.0025  # lands as a 1px hairline at every size, like CSS --rule
SUPERELLIPSE_N = 5.0  # standard approximation of the Apple squircle


def split_families(src: Path) -> tuple[Image.Image, Image.Image]:
    """Separate the two-tone mark into one alpha mask per colour."""
    im = Image.open(src).convert("RGBA")
    w, h = im.size
    a_mask, b_mask = Image.new("L", (w, h), 0), Image.new("L", (w, h), 0)
    sp, ap, bp = im.load(), a_mask.load(), b_mask.load()
    for y in range(h):
        for x in range(w):
            r, g, b, alpha = sp[x, y]
            if not alpha:
                continue
            # r > g > b is the amber ring; everything else is the cyan one.
            if r > g > b:
                ap[x, y] = alpha
            else:
                bp[x, y] = alpha
    return a_mask, b_mask


def tint(mask: Image.Image, rgb: tuple[int, int, int]) -> Image.Image:
    layer = Image.new("RGBA", mask.size, rgb + (0,))
    layer.putalpha(mask)
    return layer


def squircle(size: int, n: float = SUPERELLIPSE_N, ss: int = 4) -> Image.Image:
    """Superellipse mask, supersampled so the curve lands without stair-steps.

    Drawn one scanline at a time. Filling the span with a line keeps the work
    in Pillow's C loop; setting pixels from Python costs minutes at 4096px.
    """
    big = size * ss
    m = Image.new("L", (big, big), 0)
    d = ImageDraw.Draw(m)
    r = big / 2.0
    for y in range(big):
        dy = abs((y + 0.5) - r) / r
        if dy > 1:
            continue
        dx = (1.0 - dy**n) ** (1.0 / n) * r
        d.line([(round(r - dx), y), (round(r + dx) - 1, y)], fill=255)
    return m.resize((size, size), Image.LANCZOS)


def render(size: int, mark: Image.Image) -> Image.Image:
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    p = round(size * BODY)
    mask = squircle(p)

    plate = Image.new("RGBA", (p, p), PLATE + (255,))
    plate.putalpha(mask)

    # The rim is the mask minus a shrunken copy of itself: a hairline that
    # follows the squircle exactly instead of approximating it with a stroke.
    k = max(1, round(p * RIM_W))
    eroded = Image.new("L", (p, p), 0)
    eroded.paste(mask.resize((p - 2 * k, p - 2 * k), Image.LANCZOS), (k, k))
    rim = Image.new("RGBA", (p, p), RIM + (0,))
    rim.putalpha(ImageChops.subtract(mask, eroded))
    plate = Image.alpha_composite(plate, rim)

    off = (size - p) // 2
    canvas.alpha_composite(plate, (off, off))

    art = round(p * INSET)
    canvas.alpha_composite(
        mark.resize((art, art), Image.LANCZOS),
        (off + (p - art) // 2, off + (p - art) // 2),
    )
    return canvas


def main() -> int:
    if not MARK.exists():
        print(f"missing source artwork: {MARK}", file=sys.stderr)
        return 1

    a_mask, b_mask = split_families(MARK)
    mark = Image.alpha_composite(tint(b_mask, RING_B), tint(a_mask, RING_A))

    # Render each size from scratch rather than downsampling one master: the
    # plate and rim are procedural, so a 32px icon gets a 32px-crisp hairline
    # instead of a grey smear.
    def at(size: int) -> Image.Image:
        return render(size, mark)

    for name, size in [
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("32x32.png", 32),
    ]:
        at(size).save(HERE / name)
        print(f"  {name}")

    at(128).save(PUBLIC)
    print(f"  {PUBLIC.relative_to(HERE.parents[1])}")

    iconset = HERE / "icon.iconset"
    iconset.mkdir(exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        at(base).save(iconset / f"icon_{base}x{base}.png")
        at(base * 2).save(iconset / f"icon_{base}x{base}@2x.png")
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset), "-o", str(HERE / "icon.icns")],
        check=True,
    )
    for f in iconset.iterdir():
        f.unlink()
    iconset.rmdir()
    print("  icon.icns")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
