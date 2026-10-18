# SPDX-License-Identifier: GPL-3.0-only
# Copyright 2026 Konrad (KonBuku)
"""Generate the blurrd brand assets (logo concept "Redact").

The mark is a name-tag pill: its left part is crisp and its right part melts
into a gaussian blur, which is what blurrd does to a player name. Everything
is built from SVG primitives; the wordmark and the social-preview text are
Geist outlines, so no generated file depends on an installed font.

Usage:
    python brand.py --deps <dir> [--out <dir>] [--sheet]

``--deps`` is a folder with ``node_modules/@resvg/resvg-js`` and
``node_modules/geist`` installed (see README.md). ``--out`` defaults to the
parent folder of this script (``assets/brand``). ``--sheet`` also writes
``final-sheet.png``, a review sheet with the icon at 16/24/32/128/256 px; it
is meant for checking and is not committed.
"""

from __future__ import annotations

import argparse
import io
import struct
import subprocess
from dataclasses import dataclass
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent

INK = "#0D0D0D"
WHITE = "#FFFFFF"
NIGHT = "#161616"
PAPER = "#F8F8F8"

# Pill geometry on the 1024 unit icon canvas.
PILL_X, PILL_Y, PILL_W, PILL_H = 232, 420, 560, 184
BLUR_STD_DEV = 34


@dataclass(frozen=True)
class Tile:
    """Rounded tile on the 1024 unit canvas: position, size and corner radius."""

    x: float
    y: float
    size: float
    radius: float


# macOS grid (824/1024, 100 unit margin) and the Windows/Linux full-bleed tile.
MAC_TILE = Tile(100, 100, 824, 186)
FULL_BLEED_TILE = Tile(32, 32, 960, 216)


# --------------------------------------------------------------------------
# Text outlines
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Outline:
    """An outlined string in font units (y axis pointing down)."""

    path: str
    xmin: float
    ymin: float
    xmax: float
    ymax: float
    units_per_em: int


def outline(text: str, font_path: Path, tracking: float = 0.0) -> Outline:
    """Convert ``text`` to one SVG path; ``tracking`` is in em."""
    font = TTFont(font_path)
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    upm = font["head"].unitsPerEm
    pen = SVGPathPen(glyphs)
    bounds = BoundsPen(glyphs)
    x = 0.0
    for ch in text:
        name = cmap[ord(ch)]
        transform = (1, 0, 0, -1, x, 0)
        glyphs[name].draw(TransformPen(pen, transform))
        glyphs[name].draw(TransformPen(bounds, transform))
        x += glyphs[name].width + tracking * upm
    xmin, ymin, xmax, ymax = bounds.bounds
    return Outline(pen.getCommands(), xmin, ymin, xmax, ymax, upm)


def place_by_ink(o: Outline, x: float, y_top: float, ink_height: float, fill: str) -> tuple[str, float]:
    """Place an outline so its ink box starts at (x, y_top) with the given ink height."""
    scale = ink_height / (o.ymax - o.ymin)
    tx = x - o.xmin * scale
    ty = y_top - o.ymin * scale
    element = f'<path fill="{fill}" transform="translate({tx:.2f},{ty:.2f}) scale({scale:.5f})" d="{o.path}"/>'
    return element, (o.xmax - o.xmin) * scale


def place_centered(o: Outline, x_center: float, baseline: float, size: float, fill: str) -> str:
    """Place an outline horizontally centred on a baseline at a font size in px."""
    scale = size / o.units_per_em
    width = (o.xmax - o.xmin) * scale
    tx = x_center - width / 2 - o.xmin * scale
    return f'<path fill="{fill}" transform="translate({tx:.2f},{baseline:.2f}) scale({scale:.5f})" d="{o.path}"/>'


# --------------------------------------------------------------------------
# Building blocks
# --------------------------------------------------------------------------


def tile_shapes(uid: str, mode: str, t: Tile) -> tuple[str, str]:
    """Return (defs, shapes) for a tile with gradient fill, 10 % rim and clip path."""
    top, bottom = ("#1C1C1E", "#050505") if mode == "dark" else ("#FFFFFF", "#EFEFF1")
    rim = "rgba(255,255,255,0.10)" if mode == "dark" else "rgba(13,13,13,0.10)"
    defs = (
        f'<linearGradient id="{uid}t" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>'
        f'<clipPath id="{uid}c"><rect x="{t.x}" y="{t.y}" width="{t.size}" height="{t.size}" rx="{t.radius}"/></clipPath>'
    )
    shapes = (
        f'<rect x="{t.x}" y="{t.y}" width="{t.size}" height="{t.size}" rx="{t.radius}" fill="url(#{uid}t)"/>'
        f'<rect x="{t.x + 1.5}" y="{t.y + 1.5}" width="{t.size - 3}" height="{t.size - 3}" rx="{t.radius - 1.5}"'
        f' fill="none" stroke="{rim}" stroke-width="3"/>'
    )
    return defs, shapes


def pill(x: float = PILL_X, y: float = PILL_Y, w: float = PILL_W, h: float = PILL_H) -> str:
    """Fully rounded rectangle (the name tag)."""
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{h / 2}"/>'


def redact_glyph(uid: str, fg: str) -> tuple[str, str]:
    """Full-detail mark: a crisp pill cross-faded into a blurred copy of itself.

    Two complementary luminance masks overlap between 22 % and 70 % of the pill
    width, so the transition has no visible seam.
    """
    x0, x1 = PILL_X, PILL_X + PILL_W
    defs = (
        f'<filter id="{uid}b" x="-30%" y="-80%" width="160%" height="260%">'
        f'<feGaussianBlur stdDeviation="{BLUR_STD_DEV}"/></filter>'
        f'<linearGradient id="{uid}g" x1="{x0}" y1="0" x2="{x1}" y2="0" gradientUnits="userSpaceOnUse">'
        f'<stop offset="0.28" stop-color="#fff"/><stop offset="0.58" stop-color="#000"/></linearGradient>'
        f'<linearGradient id="{uid}gi" x1="{x0}" y1="0" x2="{x1}" y2="0" gradientUnits="userSpaceOnUse">'
        f'<stop offset="0.22" stop-color="#000"/><stop offset="0.70" stop-color="#fff"/></linearGradient>'
        f'<mask id="{uid}m" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">'
        f'<rect width="1024" height="1024" fill="url(#{uid}g)"/></mask>'
        f'<mask id="{uid}mi" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">'
        f'<rect width="1024" height="1024" fill="url(#{uid}gi)"/></mask>'
    )
    body = (
        f'<g fill="{fg}" mask="url(#{uid}m)">{pill()}</g>'
        f'<g fill="{fg}" mask="url(#{uid}mi)" filter="url(#{uid}b)">{pill()}</g>'
    )
    return defs, body


def redact_glyph_small(uid: str, fg: str) -> tuple[str, str]:
    """Simplified mark for 16-32 px: no blur filter, a taller pill that fades by opacity.

    Downsampling the gaussian version turns the blur into grey mush at these
    sizes; an opacity ramp keeps the crisp-to-soft idea readable.
    """
    x, y, w, h = 200, 382, 624, 260
    defs = (
        f'<linearGradient id="{uid}f" x1="{x}" y1="0" x2="{x + w}" y2="0" gradientUnits="userSpaceOnUse">'
        f'<stop offset="0.30" stop-color="{fg}"/><stop offset="1" stop-color="{fg}" stop-opacity="0.18"/>'
        f"</linearGradient>"
    )
    body = f'<g fill="url(#{uid}f)">{pill(x, y, w, h)}</g>'
    return defs, body


def svg_doc(width: float, height: float, view_box: str, content: str) -> str:
    """Wrap content in a standalone SVG document with an accessible title."""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" viewBox="{view_box}">'
        f"<title>blurrd</title>{content}</svg>\n"
    )


def icon_svg(mode: str, tile: Tile, small: bool = False) -> str:
    """App icon: tile plus mark on the 1024 unit canvas."""
    uid = ("s" if small else "f") + mode[0] + str(int(tile.x))
    fg = WHITE if mode == "dark" else INK
    tdefs, tshapes = tile_shapes(uid, mode, tile)
    gdefs, gbody = redact_glyph_small(uid, fg) if small else redact_glyph(uid, fg)
    content = f"<defs>{tdefs}{gdefs}</defs>{tshapes}<g clip-path=\"url(#{uid}c)\">{gbody}</g>"
    return svg_doc(1024, 1024, "0 0 1024 1024", content)


def glyph_svg(color: str) -> str:
    """Mark without a tile (title bar, sidebar, empty states).

    ``currentColor`` lets the UI colour it with ``text-foreground``. The view
    box includes the blur spread on the right.
    """
    gdefs, gbody = redact_glyph("g", color)
    return svg_doc(720, 392, "200 316 720 392", f"<defs>{gdefs}</defs>{gbody}")


def lockup_svg(word: Outline, bg: str) -> str:
    """Horizontal lockup: dark tile + wordmark (ink height 0.56 x tile, gap 0.13 x icon)."""
    uid = "l" + bg[0]
    height = 256
    s = height / 1024
    tdefs, tshapes = tile_shapes(uid, "dark", MAC_TILE)
    gdefs, gbody = redact_glyph(uid, WHITE)
    icon = f'<g transform="scale({s})">{tshapes}<g clip-path="url(#{uid}c)">{gbody}</g></g>'
    tile_left, tile_top, tile_size = MAC_TILE.x * s, MAC_TILE.y * s, MAC_TILE.size * s
    ink_h = tile_size * 0.56
    text_x = tile_left + tile_size + height * 0.13
    mark, text_w = place_by_ink(word, text_x, tile_top + (tile_size - ink_h) / 2, ink_h, INK if bg == "light" else WHITE)
    width = text_x + text_w + tile_left
    return svg_doc(width, height, f"0 0 {width:.1f} {height}", f"<defs>{tdefs}{gdefs}</defs>{icon}{mark}")


def wordmark_svg(word: Outline, fill: str) -> str:
    """Wordmark only, 160 px ink height with a 20 px margin."""
    mark, width = place_by_ink(word, 20, 20, 160, fill)
    total = width + 40
    return svg_doc(total, 200, f"0 0 {total:.1f} 200", mark)


def social_preview_svg(word: Outline, regular: Path, mono: Path) -> str:
    """GitHub social preview, 1280 x 640, solid dark background."""
    width, height = 1280, 640
    uid = "sp"
    icon_px = 168
    s = icon_px / MAC_TILE.size
    ink_h = icon_px * 0.56
    _, word_w = place_by_ink(word, 0, 0, ink_h, WHITE)
    gap = icon_px * 0.16
    x0 = (width - (icon_px + gap + word_w)) / 2
    y0 = 170
    tdefs, tshapes = tile_shapes(uid, "dark", MAC_TILE)
    gdefs, gbody = redact_glyph(uid, WHITE)
    icon = (
        f'<g transform="translate({x0 - MAC_TILE.x * s:.2f},{y0 - MAC_TILE.y * s:.2f}) scale({s:.5f})">'
        f'{tshapes}<g clip-path="url(#{uid}c)">{gbody}</g></g>'
    )
    mark, _ = place_by_ink(word, x0 + icon_px + gap, y0 + (icon_px - ink_h) / 2, ink_h, WHITE)
    tagline = place_centered(
        outline("Blur player names in gameplay recordings.", regular, -0.01), width / 2, 440, 40, "#D4D4D4"
    )
    features = place_centered(
        outline("kill feed  ·  banners  ·  chat  ·  scoreboard  ·  death recap", mono),
        width / 2,
        500,
        22,
        "#8A8A8A",
    )
    background = (
        '<radialGradient id="bg" cx="0.5" cy="0.38" r="0.75">'
        '<stop offset="0" stop-color="#202022"/><stop offset="1" stop-color="#121212"/></radialGradient>'
    )
    content = (
        f"<defs>{tdefs}{gdefs}{background}</defs>"
        f'<rect width="{width}" height="{height}" fill="url(#bg)"/>{icon}{mark}{tagline}{features}'
    )
    return svg_doc(width, height, f"0 0 {width} {height}", content)


# --------------------------------------------------------------------------
# Raster output
# --------------------------------------------------------------------------


def render(deps: Path, width: int, jobs: list[tuple[Path, Path]]) -> None:
    """Rasterise SVG files with resvg (render.mjs)."""
    args = ["node", str(HERE / "render.mjs"), str(deps), str(width)]
    for svg, png in jobs:
        png.parent.mkdir(parents=True, exist_ok=True)
        args += [str(svg), str(png)]
    subprocess.run(args, check=True)


def resized(image: Image.Image, size: int) -> Image.Image:
    """Square Lanczos resize."""
    return image.resize((size, size), Image.Resampling.LANCZOS)


def write_ico(path: Path, layers: list[Image.Image]) -> None:
    """Write an ICO with PNG-encoded layers in the given order.

    Pillow sorts layers by size; Tauri uses the first layer in development, so
    the 32 px layer has to come first.
    """
    blobs = []
    for layer in layers:
        buf = io.BytesIO()
        layer.save(buf, format="PNG")
        blobs.append(buf.getvalue())
    header = struct.pack("<HHH", 0, 1, len(layers))
    offset = 6 + 16 * len(layers)
    entries = b""
    for layer, blob in zip(layers, blobs, strict=True):
        dim = layer.width if layer.width < 256 else 0
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    path.write_bytes(header + entries + b"".join(blobs))


def review_sheet(out: Path, scratch: Path) -> None:
    """Sheet with the icon at 256/128/32 px and the small variant at 32/24/16 px (also 4x enlarged)."""
    big = Image.open(scratch / "fullbleed.png").convert("RGBA")
    small = Image.open(scratch / "small.png").convert("RGBA")
    lockup_light = Image.open(out / "png" / "blurrd-lockup-on-light.png").convert("RGBA")
    lockup_dark = Image.open(out / "png" / "blurrd-lockup-on-dark.png").convert("RGBA")
    sheet = Image.new("RGB", (1600, 900), PAPER)
    draw = ImageDraw.Draw(sheet)
    draw.rectangle([800, 0, 1600, 900], fill=NIGHT)
    for half in (0, 800):
        x = half + 40
        for size in (256, 128, 32):
            im = resized(big, size)
            sheet.paste(im, (x, 40), im)
            x += size + 30
        x = half + 40
        for size in (32, 24, 16):
            im = resized(small, size)
            sheet.paste(im, (x, 340), im)
            zoom = im.resize((size * 4, size * 4), Image.Resampling.NEAREST)
            sheet.paste(zoom, (x, 400), zoom)
            x += size * 4 + 30
        lockup = lockup_light if half == 0 else lockup_dark
        h = 120
        w = round(lockup.width * h / lockup.height)
        im = lockup.resize((w, h), Image.Resampling.LANCZOS)
        sheet.paste(im, (half + 40, 600), im)
    sheet.save(out / "final-sheet.png")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--deps", type=Path, required=True, help="folder with node_modules (resvg-js, geist)")
    parser.add_argument("--out", type=Path, default=HERE.parent, help="output folder (default: assets/brand)")
    parser.add_argument("--sheet", action="store_true", help="also write final-sheet.png for review")
    args = parser.parse_args()

    deps: Path = args.deps.resolve()
    out: Path = args.out.resolve()
    fonts = deps / "node_modules" / "geist" / "dist" / "fonts"
    semibold = fonts / "geist-sans" / "Geist-SemiBold.ttf"
    regular = fonts / "geist-sans" / "Geist-Regular.ttf"
    mono = fonts / "geist-mono" / "GeistMono-Regular.ttf"
    word = outline("blurrd", semibold, tracking=-0.035)

    svgs = {
        "blurrd-icon.svg": icon_svg("dark", MAC_TILE),
        "blurrd-icon-light.svg": icon_svg("light", MAC_TILE),
        "blurrd-icon-fullbleed.svg": icon_svg("dark", FULL_BLEED_TILE),
        "blurrd-icon-small.svg": icon_svg("dark", FULL_BLEED_TILE, small=True),
        "blurrd-glyph.svg": glyph_svg("currentColor"),
        "blurrd-glyph-ink.svg": glyph_svg(INK),
        "blurrd-glyph-white.svg": glyph_svg(WHITE),
        "blurrd-lockup-on-light.svg": lockup_svg(word, "light"),
        "blurrd-lockup-on-dark.svg": lockup_svg(word, "dark"),
        "blurrd-wordmark-ink.svg": wordmark_svg(word, INK),
        "blurrd-wordmark-white.svg": wordmark_svg(word, WHITE),
        "social-preview.svg": social_preview_svg(word, regular, mono),
    }
    out.mkdir(parents=True, exist_ok=True)
    for name, svg in svgs.items():
        (out / name).write_text(svg, encoding="utf-8", newline="\n")

    scratch = out / ".render"
    tauri = out / "tauri"
    tauri.mkdir(exist_ok=True)
    render(
        deps,
        1024,
        [
            (out / "blurrd-icon-fullbleed.svg", out / "app-icon.png"),
            (out / "blurrd-icon.svg", out / "app-icon-macos.png"),
            (out / "blurrd-icon-fullbleed.svg", scratch / "fullbleed.png"),
            (out / "blurrd-icon-light.svg", out / "png" / "blurrd-icon-light.png"),
            (out / "blurrd-lockup-on-light.svg", out / "png" / "blurrd-lockup-on-light.png"),
            (out / "blurrd-lockup-on-dark.svg", out / "png" / "blurrd-lockup-on-dark.png"),
        ],
    )
    render(deps, 256, [(out / "blurrd-icon-small.svg", scratch / "small.png")])
    render(deps, 1280, [(out / "social-preview.svg", out / "social-preview.png")])

    big = Image.open(scratch / "fullbleed.png").convert("RGBA")
    small = Image.open(scratch / "small.png").convert("RGBA")
    resized(small, 32).save(tauri / "32x32.png")
    resized(big, 128).save(tauri / "128x128.png")
    resized(big, 256).save(tauri / "128x128@2x.png")
    layers = {16: small, 24: small, 32: small, 48: big, 64: big, 256: big}
    write_ico(tauri / "icon.ico", [resized(layers[s], s) for s in (32, 16, 24, 48, 64, 256)])
    Image.open(out / "app-icon-macos.png").convert("RGBA").save(tauri / "icon.icns", format="ICNS")

    if args.sheet:
        review_sheet(out, scratch)
    for f in scratch.iterdir():
        f.unlink()
    scratch.rmdir()


if __name__ == "__main__":
    main()
