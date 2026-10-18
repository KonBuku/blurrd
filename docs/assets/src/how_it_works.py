# SPDX-License-Identifier: GPL-3.0-only
# Copyright 2026 Konrad (KonBuku)
"""Generate the "How it works" diagram (docs/assets/how-it-works-*.svg).

The text is converted to outlines with HarfBuzz and fontTools, so the SVG
looks the same in every browser and needs no font. Inter (OFL-1.1) is
read from a local folder and never committed:

    python docs/assets/src/how_it_works.py --fonts <folder with Inter-*.ttf>

Requires: fonttools, uharfbuzz. The output is deterministic.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path

import uharfbuzz as hb
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

OUT = Path(__file__).resolve().parents[1]
W, H = 1600, 768

THEMES = {
    "dark": dict(
        bg="#121212",
        bg2="#1b1b1b",
        card="#202020",
        card_hi="#262626",
        border="rgba(255,255,255,0.10)",
        border_hi="rgba(255,255,255,0.16)",
        fg="#ededed",
        sub="rgba(212,212,212,0.70)",
        faint="rgba(212,212,212,0.42)",
        line="rgba(255,255,255,0.28)",
        accent="#4099ff",
        accent_fg="#80beff",
        accent_bg="#0b2440",
        ok="#80beff",
        ok_bg="#0b2440",
        chip="rgba(255,255,255,0.04)",
    ),
    "light": dict(
        bg="#f6f6f7",
        bg2="#ffffff",
        card="#ffffff",
        card_hi="#ffffff",
        border="rgba(13,13,13,0.10)",
        border_hi="rgba(13,13,13,0.14)",
        fg="#1f1f1f",
        sub="rgba(38,38,38,0.72)",
        faint="rgba(38,38,38,0.45)",
        line="rgba(13,13,13,0.30)",
        accent="#0b7fff",
        accent_fg="#0066dd",
        accent_bg="#ebf4ff",
        ok="#0066dd",
        ok_bg="#ebf4ff",
        chip="rgba(13,13,13,0.025)",
    ),
}

# Lucide icons (ISC licence), 24x24 grid, drawn with stroke width 1.5.
ICONS = {
    "film": [
        '<rect x="3" y="3" width="18" height="18" rx="2"/>',
        '<path d="M7 3v18M3 7.5h4M3 12h18M3 16.5h4M17 3v18M17 7.5h4M17 16.5h4"/>',
    ],
    "cpu": [
        '<rect x="4" y="4" width="16" height="16" rx="2"/>',
        '<rect x="9" y="9" width="6" height="6" rx="1"/>',
        '<path d="M15 2v2M15 20v2M2 15h2M2 9h2M20 15h2M20 9h2M9 2v2M9 20v2"/>',
    ],
    "scan": [
        '<path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/>',
        '<rect x="7" y="7" width="10" height="10" rx="1"/>',
        '<path d="M7 12h10M12 7v10"/>',
    ],
    "crosshair": [
        '<circle cx="12" cy="12" r="10"/>',
        '<path d="M22 12h-4M6 12H2M12 6V2M12 22v-4"/>',
    ],
    "blur": [
        '<rect x="3" y="5" width="18" height="14" rx="3"/>',
        '<path d="M7 10h4M7 14h7M14 10h3" stroke-dasharray="1.5 2"/>',
    ],
    "scan-text": [
        '<path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/>',
        '<path d="M7 8h8M7 12h10M7 16h6"/>',
    ],
    "badge-check": [
        '<path d="M3.85 8.62a4 4 0 0 1 4.78-4.77 4 4 0 0 1 6.74 0 4 4 0 0 1 4.78 4.78'
        " 4 4 0 0 1 0 6.74 4 4 0 0 1-4.77 4.78 4 4 0 0 1-6.75 0 4 4 0 0 1-4.78-4.77"
        ' 4 4 0 0 1 0-6.76Z"/>',
        '<path d="m9 12 2 2 4-4"/>',
    ],
    "shield-check": [
        '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6'
        "a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5"
        ' 19 5a1 1 0 0 1 1 1z"/>',
        '<path d="m9 12 2 2 4-4"/>',
    ],
    "cloud-off": [
        '<path d="m2 2 20 20M5.782 5.782A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.307-.193'
        'M21.532 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7.008 7.008 0 0 0 10 5.07"/>',
    ],
    "refresh": [
        '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5'
        'M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5"/>',
    ],
}


@dataclass
class Font:
    tt: TTFont
    hb_font: hb.Font
    upem: int

    @classmethod
    def load(cls, path: Path) -> Font:
        blob = hb.Blob.from_file_path(str(path))
        face = hb.Face(blob)
        return cls(TTFont(str(path)), hb.Font(face), face.upem)

    def shape(self, text: str):
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(self.hb_font, buf, {"kern": True, "liga": True})
        return buf.glyph_infos, buf.glyph_positions

    def width(self, text: str, size: float, tracking: float = 0.0) -> float:
        _, pos = self.shape(text)
        return sum(p.x_advance for p in pos) * size / self.upem + tracking * size * len(pos)

    def glyph_d(self, name: str) -> str:
        """Outline of one glyph in font units, y pointing down."""
        glyphs = self.tt.getGlyphSet()
        pen = SVGPathPen(glyphs, ntos=lambda v: str(round(v)))
        glyphs[name].draw(TransformPen(pen, (1, 0, 0, -1, 0, 0)))
        return pen.getCommands()

    def place(self, text: str, x: float, y: float, size: float, tracking: float = 0.0):
        """(glyph name, x, y) of every glyph of `text` set at (x, y)."""
        infos, pos = self.shape(text)
        order = self.tt.getGlyphOrder()
        scale = size / self.upem
        cx = x
        for info, p in zip(infos, pos):
            yield order[info.codepoint], cx + p.x_offset * scale, y - p.y_offset * scale
            cx += p.x_advance * scale + tracking * size


class Svg:
    def __init__(self, theme: dict, fonts: dict[str, Font]):
        self.t = theme
        self.f = fonts
        self.parts: list[str] = []
        self.glyphs: dict[str, str] = {}

    def add(self, s: str) -> None:
        self.parts.append(s)

    def text(self, s, x, y, size, weight="regular", fill=None, anchor="start", tracking=0.0):
        """Outlined text: each glyph is defined once and placed with <use>."""
        font = self.f[weight]
        if anchor != "start":
            w = font.width(s, size, tracking)
            x = x - (w / 2 if anchor == "middle" else w)
        scale = size / font.upem
        uses = []
        for name, gx, gy in font.place(s, x, y, size, tracking):
            gid = f"{weight[0]}-{name}".replace(".", "_")
            if gid not in self.glyphs:
                d = font.glyph_d(name)
                if not d:
                    continue
                self.glyphs[gid] = f'<path id="{gid}" d="{d}"/>'
            uses.append(f'<use href="#{gid}" x="{gx / scale:.0f}" y="{gy / scale:.0f}"/>')
        self.add(f'<g fill="{fill or self.t["fg"]}" transform="scale({scale:.6f})">{"".join(uses)}</g>')

    def icon(self, name, x, y, size, color):
        s = size / 24
        body = "".join(ICONS[name])
        self.add(
            f'<g transform="translate({x} {y}) scale({s:.4f})" fill="none" stroke="{color}" '
            f'stroke-width="{1.5:.2f}" stroke-linecap="round" stroke-linejoin="round">{body}</g>'
        )


def card(svg: Svg, x, y, w, h, *, hi=False, stroke=None):
    t = svg.t
    svg.add(
        f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="18" fill="{t["card_hi"] if hi else t["card"]}" '
        f'stroke="{stroke or (t["border_hi"] if hi else t["border"])}"/>'
    )


def tag(svg: Svg, x, y, label):
    t = svg.t
    w = svg.f["medium"].width(label, 12.5) + 24
    svg.add(
        f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="26" rx="13" fill="{t["chip"]}" stroke="{t["border"]}"/>'
    )
    svg.text(label, x + 12, y + 17.5, 12.5, "medium", t["sub"])


def step(svg: Svg, x, y, w, h, num, icon, title, lines, *, tone="accent", detail=None):
    """A pipeline card: icon tile, step number, title and two short lines."""
    t = svg.t
    card(svg, x, y, w, h)
    fg, bg = (t["ok"], t["ok_bg"]) if tone == "ok" else (t["accent_fg"], t["accent_bg"])
    svg.add(f'<rect x="{x + 22}" y="{y + 22}" width="44" height="44" rx="12" fill="{bg}"/>')
    svg.icon(icon, x + 32, y + 32, 24, fg)
    svg.text(f"{num:02d}", x + w - 24, y + 42, 13, "medium", t["faint"], anchor="end", tracking=0.06)
    svg.text(title, x + 22, y + 100, 19, "semibold")
    for i, line in enumerate(lines):
        svg.text(line, x + 22, y + 128 + i * 22, 14.5, "regular", t["sub"])
    if detail:
        tag(svg, x + 22, y + h - 48, detail)


def arrow(svg: Svg, d: str, end: tuple[float, float], direction: str, *, color=None, dash=False):
    """A connector with a chevron head at `end`, pointing right, down or up."""
    t = svg.t
    c = color or t["line"]
    da = ' stroke-dasharray="5 6"' if dash else ""
    style = f'fill="none" stroke="{c}" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"'
    svg.add(f'<path d="{d}" {style}{da}/>')
    x, y = end
    k = 6
    head = {
        "right": f"M{x - k:.1f} {y - k:.1f}L{x:.1f} {y:.1f}L{x - k:.1f} {y + k:.1f}",
        "down": f"M{x - k:.1f} {y - k:.1f}L{x:.1f} {y:.1f}L{x + k:.1f} {y - k:.1f}",
        "up": f"M{x - k:.1f} {y + k:.1f}L{x:.1f} {y:.1f}L{x + k:.1f} {y + k:.1f}",
    }[direction]
    svg.add(f'<path d="{head}" {style}/>')


def build(theme_name: str, fonts: dict[str, Font]) -> str:
    t = THEMES[theme_name]
    svg = Svg(t, fonts)

    # Row 1: recording -> decode -> calibration -> detectors.
    y1, h1 = 24, 248
    cw = 262
    gap = 48
    x = 64
    step(
        svg,
        x,
        y1,
        cw,
        h1,
        1,
        "film",
        "Your recording",
        ["MP4, MOV or MKV, 720p to 4K,", "H.264, HEVC or AV1"],
        detail="Read once, never changed",
    )
    x2 = x + cw + gap
    step(
        svg,
        x2,
        y1,
        cw,
        h1,
        2,
        "cpu",
        "GPU decode",
        ["NVDEC, frames decoded once", "and shared by every worker"],
        detail="Fallback: D3D11VA, software",
    )
    x3 = x2 + cw + gap
    step(
        svg,
        x3,
        y1,
        cw,
        h1,
        3,
        "scan",
        "HUD calibration",
        ["Finds the layout: 16:9, 4:3,", "16:10, stretched or boxed"],
        detail="Structure, not pixel positions",
    )
    x4 = x3 + cw + gap
    w4 = W - 64 - x4
    card(svg, x4, y1, w4, h1)
    svg.add(f'<rect x="{x4 + 22}" y="{y1 + 22}" width="44" height="44" rx="12" fill="{t["accent_bg"]}"/>')
    svg.icon("crosshair", x4 + 32, y1 + 32, 24, t["accent_fg"])
    svg.text("04", x4 + w4 - 24, y1 + 42, 13, "medium", t["faint"], anchor="end", tracking=0.06)
    svg.text("Per-region detectors", x4 + 84, y1 + 42, 19, "semibold")
    svg.text("Track every name from its first to its last frame", x4 + 84, y1 + 64, 14.5, "regular", t["sub"])
    chips = [
        ("Kill feed", t["faint"]),
        ("Round banner", t["faint"]),
        ("Voice chat", t["faint"]),
        ("Text chat", t["faint"]),
        ("Scoreboard", t["faint"]),
        ("Death recap", t["faint"]),
        ("Spectating", t["faint"]),
        ("Match end", t["faint"]),
    ]
    cols = 2
    chip_w = (w4 - 44 - 12) / cols
    chip_h = 30
    for i, (label, color) in enumerate(chips):
        cx = x4 + 22 + (i % cols) * (chip_w + 12)
        cy = y1 + 94 + (i // cols) * (chip_h + 8)
        svg.add(
            f'<rect x="{cx:.1f}" y="{cy}" width="{chip_w:.1f}" height="{chip_h}" rx="8" '
            f'fill="{t["chip"]}" stroke="{t["border"]}"/>'
        )
        svg.add(f'<rect x="{cx + 12:.1f}" y="{cy + 9}" width="12" height="12" rx="3" fill="{color}"/>')
        svg.text(label, cx + 34, cy + 20, 14, "medium", t["fg"])

    mid1 = y1 + 46
    for xa in (x + cw, x2 + cw, x3 + cw):
        arrow(svg, f"M{xa + 8} {mid1}H{xa + gap - 10}", (xa + gap - 10, mid1), "right")

    # Row 2: blur boxes -> encode -> OCR verification -> export.
    y2, h2 = 360, 232
    cw2 = (W - 128 - 3 * gap) / 4
    xs = [64 + i * (cw2 + gap) for i in range(4)]
    step(
        svg,
        xs[0],
        y2,
        cw2,
        h2,
        5,
        "blur",
        "Blur boxes",
        ["Soft-edged, tight to each row,", "only inside the boxes"],
        detail="YUV planes, microseconds per box",
    )
    step(
        svg,
        xs[1],
        y2,
        cw2,
        h2,
        6,
        "cpu",
        "GPU encode",
        ["NVENC, same resolution, fps,", "timestamps and audio"],
        detail="Fallback: AMF, Quick Sync, MF",
    )
    step(
        svg,
        xs[2],
        y2,
        cw2,
        h2,
        7,
        "scan-text",
        "OCR verification",
        ["Reads the blurred frames as", "exported, adds missed boxes"],
        detail="PP-OCRv5 on DirectML or CPU",
    )
    step(
        svg,
        xs[3],
        y2,
        cw2,
        h2,
        8,
        "badge-check",
        "Verified export",
        ["Clean, or named *_UNVERIFIED", "until you have checked it"],
        tone="ok",
        detail="Audio copied packet for packet",
    )
    mid2 = y2 + 46
    for i in range(3):
        xa = xs[i] + cw2
        color = t["ok"] if i == 2 else None
        arrow(svg, f"M{xa + 8:.1f} {mid2}H{xa + gap - 10:.1f}", (xa + gap - 10, mid2), "right", color=color)

    # Elbow from the detectors down to the blur boxes.
    sx = x4 + w4 / 2
    by = (y1 + h1 + y2) / 2
    tx = xs[0] + cw2 / 2
    rr = 16
    arrow(
        svg,
        f"M{sx:.1f} {y1 + h1 + 8}V{by - rr:.1f}Q{sx:.1f} {by:.1f} {sx - rr:.1f} {by:.1f}"
        f"H{tx + rr:.1f}Q{tx:.1f} {by:.1f} {tx:.1f} {by + rr:.1f}V{y2 - 10}",
        (tx, y2 - 10),
        "down",
    )

    # Fix loop: verification -> blur boxes, under row 2.
    lx0 = xs[2] + cw2 / 2
    lx1 = xs[0] + cw2 / 2
    ly = y2 + h2 + 44
    arrow(
        svg,
        f"M{lx0:.1f} {y2 + h2 + 8}V{ly - rr:.1f}Q{lx0:.1f} {ly} {lx0 - rr:.1f} {ly}"
        f"H{lx1 + rr:.1f}Q{lx1:.1f} {ly} {lx1:.1f} {ly - rr:.1f}V{y2 + h2 + 10}",
        (lx1, y2 + h2 + 10),
        "up",
        color=t["accent"],
        dash=True,
    )
    label = "Readable name found: fix boxes, read again (up to 3 rounds)"
    lw = fonts["medium"].width(label, 14) + 56
    lcx = (lx0 + lx1) / 2
    svg.add(
        f'<rect x="{lcx - lw / 2:.1f}" y="{ly - 17}" width="{lw:.1f}" height="34" rx="17" '
        f'fill="{t["card"]}" stroke="{t["accent"]}" stroke-opacity="0.55"/>'
    )
    svg.icon("refresh", lcx - lw / 2 + 16, ly - 9, 18, t["accent_fg"])
    svg.text(label, lcx - lw / 2 + 42, ly + 5, 14, "medium", t["accent_fg"])

    # Footer: privacy band.
    fy = 728
    svg.add(f'<line x1="64" y1="{fy - 30}" x2="{W - 64}" y2="{fy - 30}" stroke="{t["border"]}"/>')
    svg.icon("cloud-off", 64, fy - 6, 22, t["sub"])
    svg.text(
        "100 % local: no upload, no account, no telemetry. Reports and logs never contain the names OCR read.",
        98,
        fy + 11,
        15,
        "regular",
        t["sub"],
    )

    head = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W - 124}" height="{H - 34}" viewBox="62 22 {W - 124} {H - 34}" '
        f'role="img" aria-labelledby="title desc">'
        f'<title id="title">How blurrd works</title>'
        f'<desc id="desc">Recording, GPU decode with NVDEC, HUD layout calibration, per-region '
        f"detectors for kill feed, round banner, voice chat, text chat, scoreboard, death recap, "
        f"spectating and match end, blur boxes, GPU encode with NVENC, OCR verification that "
        f"re-reads the blurred frames and adds fix boxes, verified export. Everything runs locally."
        f"</desc><defs>{''.join(svg.glyphs.values())}</defs>"
    )
    return head + "".join(svg.parts) + "</svg>\n"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--fonts", type=Path, required=True, help="folder with Inter-*.ttf (OFL-1.1)")
    args = ap.parse_args()
    fonts = {
        w: Font.load(args.fonts / f"Inter-{n}.ttf")
        for w, n in (("regular", "Regular"), ("medium", "Medium"), ("semibold", "SemiBold"))
    }
    for name in THEMES:
        out = OUT / f"how-it-works-{name}.svg"
        out.write_text(build(name, fonts), encoding="utf-8", newline="\n")
        print(out)


if __name__ == "__main__":
    main()
