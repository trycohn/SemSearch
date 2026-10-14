"""Generates shell/icons/* (app icon + tray icons) with Pillow.

Run: .venv\\Scripts\\python.exe shell\\tools\\make_icons.py
App icon: rounded square in accent #005fb8 with a white magnifier
(design glyph "M10.5 4a6.5 6.5 0 1 0 0 13a6.5 6.5 0 0 0 0-13z M15.5 15.5L20 20", 24-unit viewBox).
Tray icon: the same glyph in the light accent #73a7d8 (as in design/Tray.dc.html), transparent background,
rendered per size so it stays crisp on a dark taskbar.
"""
from pathlib import Path

from PIL import Image, ImageDraw

ACCENT = (0x00, 0x5F, 0xB8, 255)
TRAY = (0x73, 0xA7, 0xD8, 255)
WHITE = (255, 255, 255, 255)
SS = 8  # supersampling factor
OUT = Path(__file__).resolve().parent.parent / "icons"


def glyph(draw: ImageDraw.ImageDraw, size_px: float, glyph_frac: float, stroke_units: float, color, ox=0.0, oy=0.0):
    """Draw the magnifier. 24-unit glyph bbox [4..20] is scaled to glyph_frac of size_px and centred."""
    scale = glyph_frac * size_px / 16.0
    off = size_px / 2.0 - 12.0 * scale

    def p(x, y):
        return (ox + off + x * scale, oy + off + y * scale)

    sw = stroke_units * scale
    cx, cy = p(10.5, 10.5)
    r = 6.5 * scale
    draw.ellipse((cx - r, cy - r, cx + r, cy + r), outline=color, width=max(1, round(sw)))
    (x1, y1), (x2, y2) = p(15.5, 15.5), p(20, 20)
    draw.line((x1, y1, x2, y2), fill=color, width=max(1, round(sw)))
    for (x, y) in ((x2, y2),):  # round cap at the free end of the handle
        draw.ellipse((x - sw / 2, y - sw / 2, x + sw / 2, y + sw / 2), fill=color)


def app_icon(size: int) -> Image.Image:
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = round(big * (0.03 if size <= 24 else 0.06))
    d.rounded_rectangle((m, m, big - 1 - m, big - 1 - m), radius=round(big * 0.22), fill=ACCENT)
    stroke = 2.6 if size <= 24 else 2.2
    glyph(d, big, 0.62 if size <= 24 else 0.58, stroke, WHITE)
    return img.resize((size, size), Image.LANCZOS)


def tray_icon(size: int) -> Image.Image:
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    glyph(d, big, 0.92, 2.5 if size <= 20 else 2.3, TRAY)
    return img.resize((size, size), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    ico_sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    imgs = [app_icon(s) for s in ico_sizes]
    imgs[-1].save(OUT / "icon.ico", format="ICO", sizes=[(s, s) for s in ico_sizes], append_images=imgs[:-1])
    app_icon(32).save(OUT / "32x32.png")
    app_icon(128).save(OUT / "128x128.png")
    app_icon(256).save(OUT / "128x128@2x.png")
    app_icon(512).save(OUT / "icon.png")
    for s in (16, 20, 24, 32):
        tray_icon(s).save(OUT / f"tray-{s}.png")
    print("icons ->", OUT)


if __name__ == "__main__":
    main()
