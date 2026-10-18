# SPDX-License-Identifier: GPL-3.0-only
# Copyright 2026 Konrad (KonBuku)
"""Generate the "How it works" diagram (docs/assets/how-it-works-*.svg).

The diagram follows docs/architecture.md: the front ends, the analysis
pass, the verification with its fix loop and the render pass, each stage
labelled with the crate that implements it, plus the speed measured for
the README.

The text is converted to outlines with HarfBuzz and fontTools, so the SVG
looks the same in every browser and needs no font. Inter and JetBrains
Mono (both OFL-1.1) are read from a local folder and never committed:

    python docs/assets/src/how_it_works.py --fonts <folder with the TTFs>

The folder needs Inter-{Regular,Medium,SemiBold}.ttf and
JetBrainsMono-{Regular,Medium}.ttf. Requires: fonttools, uharfbuzz. The
output is deterministic.
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
W = 1800

THEMES = {
    "dark": dict(
        card="#1c1c1e",
        border="rgba(255,255,255,0.12)",
        fg="#ededed",
        sub="rgba(222,222,222,0.72)",
        faint="rgba(222,222,222,0.56)",
        line="rgba(222,222,222,0.50)",
        accent="#4c9dff",
        accent_fg="#8cc2ff",
        chip="rgba(255,255,255,0.05)",
        rule="rgba(255,255,255,0.12)",
    ),
    "light": dict(
        card="#ffffff",
        border="rgba(13,13,13,0.14)",
        fg="#1f1f1f",
        sub="rgba(38,38,38,0.74)",
        faint="rgba(38,38,38,0.58)",
        line="rgba(38,38,38,0.52)",
        accent="#0b6fe0",
        accent_fg="#0a5fc2",
        chip="rgba(13,13,13,0.035)",
        rule="rgba(13,13,13,0.12)",
    ),
}

# Font keys: Inter regular, medium, semibold; JetBrains Mono regular, medium.
FONT_FILES = {
    "r": "Inter-Regular.ttf",
    "m": "Inter-Medium.ttf",
    "s": "Inter-SemiBold.ttf",
    "c": "JetBrainsMono-Regular.ttf",
    "cm": "JetBrainsMono-Medium.ttf",
}

# Layout grid: five columns.
X0 = 40
CW = 293
GAP = (W - 2 * X0 - 5 * CW) / 4
PAD = 20

TITLE = 22
BODY = 18.5
MONO = 15.5
LABEL = 15
EDGE = 14
LEAD = 25


def col(i: float) -> float:
    return X0 + i * (CW + GAP)


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
        hb.shape(self.hb_font, buf, {"kern": True, "liga": False, "calt": False})
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
        self.overflow: list[str] = []

    def add(self, s: str) -> None:
        self.parts.append(s)

    def width(self, s: str, size: float, font: str = "r", tracking: float = 0.0) -> float:
        return self.f[font].width(s, size, tracking)

    def check(self, s: str, w: float, max_w: float | None) -> None:
        if max_w is not None and w > max_w + 0.5:
            self.overflow.append(f"{s!r}: {w:.0f} > {max_w:.0f}")

    def text(self, s, x, y, size, font="r", fill=None, anchor="start", tracking=0.0, max_w=None):
        """Outlined text: each glyph is defined once and placed with <use>."""
        f = self.f[font]
        w = f.width(s, size, tracking)
        self.check(s, w, max_w)
        if anchor != "start":
            x = x - (w / 2 if anchor == "middle" else w)
        scale = size / f.upem
        uses = []
        for name, gx, gy in f.place(s, x, y, size, tracking):
            gid = f"{font}-{name}".replace(".", "_")
            if gid not in self.glyphs:
                d = f.glyph_d(name)
                if not d:
                    continue
                self.glyphs[gid] = f'<path id="{gid}" d="{d}"/>'
            uses.append(f'<use href="#{gid}" x="{gx / scale:.0f}" y="{gy / scale:.0f}"/>')
        if uses:
            self.add(f'<g fill="{fill or self.t["fg"]}" transform="scale({scale:.6f})">{"".join(uses)}</g>')
        return w

    def rich(self, s, x, y, size, fill, max_w=None):
        """A line in which `code` spans are set in the monospace font."""
        cx = x
        for i, part in enumerate(s.split("`")):
            if part:
                mono = i % 2 == 1
                cx += self.text(part, cx, y, size * 0.9 if mono else size, "c" if mono else "r", fill)
        self.check(s, cx - x, max_w)
        return cx - x


def rect(svg: Svg, x, y, w, h, *, stroke=None):
    svg.add(
        f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="12" '
        f'fill="{svg.t["card"]}" stroke="{stroke or svg.t["border"]}" stroke-width="1.25"/>'
    )


def chip(svg: Svg, x_right, y, label):
    """A small monospace pill, right-aligned at `x_right`, top at `y`."""
    t = svg.t
    w = svg.width(label, EDGE, "c") + 18
    svg.add(
        f'<rect x="{x_right - w:.1f}" y="{y}" width="{w:.1f}" height="26" rx="13" '
        f'fill="{t["chip"]}" stroke="{t["border"]}" stroke-width="1"/>'
    )
    svg.text(label, x_right - w + 9, y + 18, EDGE, "c", t["sub"])


def node(svg: Svg, x, y, w, h, crate, title, lines, *, out=None, badge=None, accent=False):
    """A stage: crate (mono), optional badge, title, body and its output."""
    t = svg.t
    rect(svg, x, y, w, h, stroke=t["accent"] if accent else None)
    room = w - 2 * PAD
    if badge:
        room -= svg.width(badge, EDGE, "c") + 24
        chip(svg, x + w - 14, y + 14, badge)
    svg.text(crate, x + PAD, y + 32, MONO, "c", t["accent_fg"] if accent else t["faint"], max_w=room)
    svg.text(title, x + PAD, y + 66, TITLE, "s", max_w=w - 2 * PAD)
    for i, line in enumerate(lines):
        svg.rich(line, x + PAD, y + 98 + i * LEAD, BODY, t["sub"], max_w=w - 2 * PAD)
    if out:
        ly = y + h - 46
        svg.add(f'<line x1="{x + 1:.1f}" y1="{ly}" x2="{x + w - 1:.1f}" y2="{ly}" stroke="{t["border"]}"/>')
        arrow_w = svg.text("→ ", x + PAD, y + h - 17, MONO, "c", t["faint"])
        svg.text(out, x + PAD + arrow_w, y + h - 17, MONO, "cm", t["fg"], max_w=w - 2 * PAD - arrow_w)


def section(svg: Svg, x, y, label, note=None, *, anchor="start"):
    t = svg.t
    w = svg.width(label, LABEL, "s", 0.09)
    total = w + (12 + svg.width(note, LABEL + 0.5) if note else 0)
    if anchor == "end":
        x -= total
    svg.text(label, x, y, LABEL, "s", t["fg"], tracking=0.09)
    if note:
        svg.text(note, x + w + 12, y, LABEL + 0.5, "r", t["faint"])


def arrow(svg: Svg, pts, *, color=None, r=10):
    """An orthogonal connector through `pts`, rounded corners, filled head."""
    c = color or svg.t["line"]
    (xa, ya), (xb, yb) = pts[-2], pts[-1]
    dx, dy = (xb > xa) - (xb < xa), (yb > ya) - (yb < ya)
    k = 9
    body = pts[:-1] + [(xb - dx * k * 0.7, yb - dy * k * 0.7)]
    d = f"M{body[0][0]:.1f} {body[0][1]:.1f}"
    for i in range(1, len(body) - 1):
        (px, py), (cx, cy), (nx, ny) = body[i - 1], body[i], body[i + 1]
        ux, uy = (cx > px) - (cx < px), (cy > py) - (cy < py)
        vx, vy = (nx > cx) - (nx < cx), (ny > cy) - (ny < cy)
        d += f"L{cx - ux * r:.1f} {cy - uy * r:.1f}Q{cx:.1f} {cy:.1f} {cx + vx * r:.1f} {cy + vy * r:.1f}"
    d += f"L{body[-1][0]:.1f} {body[-1][1]:.1f}"
    svg.add(f'<path d="{d}" fill="none" stroke="{c}" stroke-width="1.5" stroke-linejoin="round"/>')
    px, py = -dy, dx
    hw = 4.5
    bx, by = xb - dx * k, yb - dy * k
    svg.add(
        f'<path d="M{xb:.1f} {yb:.1f}L{bx + px * hw:.1f} {by + py * hw:.1f}'
        f'L{bx - px * hw:.1f} {by - py * hw:.1f}Z" fill="{c}"/>'
    )


def edge_label(svg: Svg, x, y, s, *, anchor="middle", color=None, pill=False):
    t = svg.t
    w = svg.width(s, EDGE, "c")
    x0 = x - (w / 2 if anchor == "middle" else (w if anchor == "end" else 0))
    if pill:
        svg.add(
            f'<rect x="{x0 - 10:.1f}" y="{y - 16:.1f}" width="{w + 20:.1f}" height="24" rx="12" '
            f'fill="{t["card"]}" stroke="{color or t["border"]}" stroke-width="1"/>'
        )
    svg.text(s, x0, y, EDGE, "c", color or t["faint"])


def build(theme_name: str, fonts: dict[str, Font]) -> tuple[str, list[str]]:
    t = THEMES[theme_name]
    svg = Svg(t, fonts)
    span2 = 2 * CW + GAP

    # Band 0: both front ends call the engine's jobs.
    y0, h0 = 50, 132
    section(svg, X0, y0 - 22, "FRONT ENDS", "both run the same jobs and report the same JobEvent stream")
    node(
        svg,
        col(0),
        y0,
        span2,
        h0,
        "apps/desktop/src-tauri (blurrd-app)",
        "Desktop app: Tauri 2, React 19",
        [
            "Tauri commands with ts-rs typed payloads, `Channel<JobEvent>`;",
            "loopback media server and `blurrd://` for previews and patches",
        ],
    )
    node(svg, col(2), y0, CW, h0, "blurrd-engine", "Jobs", ["`analyze` `run` `render`", "`reverify_ranges` `audit`"])
    node(
        svg,
        col(3),
        y0,
        span2,
        h0,
        "blurrd-cli",
        "Command-line tool",
        [
            "`blurrd probe | analyze | render | run | audit`",
            "`--json` prints the events the app gets on its job channel",
        ],
    )
    my = y0 + 48
    arrow(svg, [(col(0) + span2, my), (col(2), my)])
    arrow(svg, [(col(3), my), (col(2) + CW, my)])

    # Row 1: analysis, left to right.
    y1, h1 = 244, 208
    section(svg, X0, y1 - 22, "ANALYSIS", "Engine::analyze · bounded channels between all threads")
    node(
        svg,
        col(0),
        y1,
        CW,
        h1,
        "input",
        "Recording",
        ["MP4, MOV or MKV;", "H.264, HEVC or AV1;", "720p to 4K, SDR or HDR"],
        out="read, never changed",
    )
    node(
        svg,
        col(1),
        y1,
        CW,
        h1,
        "blurrd-video, -detect",
        "Probe + calibrate",
        ["packet scan for timestamps;", "keyframe seek, 1 per 4 s;", "16:9, 4:3, 16:10, letterbox"],
        out="PTS table, HudLayout",
    )
    node(
        svg,
        col(2),
        y1,
        CW,
        h1,
        "blurrd-video",
        "Decode once",
        ["NVDEC, D3D11VA, software;", "reference-counted frames", "shared by workers and OCR"],
        out="NV12 / P010 frames",
        badge="1 decoder",
    )
    node(
        svg,
        col(3),
        y1,
        CW,
        h1,
        "blurrd-detect",
        "Crop + detect",
        ["YUV crops of HUD regions", "→ RGB; detectors on pixel", "classes and morphology"],
        out="per-frame detections",
        badge="2–6 workers",
    )
    node(
        svg,
        col(4),
        y1,
        CW,
        h1,
        "blurrd-detect",
        "Temporal models",
        ["kill feed, banner, voice,", "chat, 4 screens; scene cuts;", "boxes final in ≤ 240 frames"],
        out="box tracks",
        badge="1 thread",
    )
    ay = y1 + 48
    for i in range(4):
        arrow(svg, [(col(i) + CW, ay), (col(i + 1), ay)])

    # Row 2: project cache, OCR pass 0, verification, box tracks.
    y2, h2 = 544, 184
    section(svg, col(3), y2 - 22, "VERIFICATION", "pass 0 runs live, the check after the last frame")
    rect(svg, col(0), y2, span2, h2)
    svg.text("%LOCALAPPDATA%\\blurrd\\projects\\<fingerprint>\\", col(0) + PAD, y2 + 32, MONO, "c", t["faint"])
    svg.text("Project cache", col(0) + PAD, y2 + 66, TITLE, "s")
    files = [
        ("project.json", "tracks, edits, verify state"),
        ("pts.bin", "timestamp of every frame"),
        ("structure.bin", "HUD signals, zones pass 0 read"),
        ("verify-<rev>.json", "boxes and counts, never text"),
    ]
    for i, (name, what) in enumerate(files):
        cx = col(0) + PAD + (i % 2) * (span2 / 2)
        cy = y2 + 102 + (i // 2) * 44
        svg.text(name, cx, cy, MONO, "cm", t["fg"])
        svg.text(what, cx, cy + 21, 15.5, "r", t["sub"], max_w=span2 / 2 - PAD - 8)
    node(
        svg,
        col(2),
        y2,
        CW,
        h2,
        "blurrd-ocr",
        "OCR pass 0",
        ["PP-OCRv5 det + rec, ONNX", "Runtime on DirectML or CPU"],
        out="zone reads, names",
        badge="≤ 4 readers",
    )
    node(
        svg,
        col(3),
        y2,
        CW,
        h2,
        "blurrd-engine, -ocr",
        "Verify + fix",
        ["blur uncovered name parts", "in memory, read them again"],
        out="OcrFix boxes, ≤ 3 rounds",
        accent=True,
    )
    node(
        svg,
        col(4),
        y2,
        CW,
        h2,
        "blurrd-core",
        "Box tracks",
        ["tight boxes when tracked,", "slot boxes when unsure"],
        out="BlurBox list per frame",
    )
    px1 = col(1) + CW / 2
    arrow(svg, [(px1, y1 + h1), (px1, y2)])
    edge_label(svg, px1 + 12, (y1 + h1 + y2) / 2 + 5, "pts.bin", anchor="start")
    dx = col(2) + CW / 2
    arrow(svg, [(dx, y1 + h1), (dx, y2)])
    edge_label(svg, dx + 12, (y1 + h1 + y2) / 2 + 5, "frames", anchor="start")
    bx = col(4) + CW / 2
    arrow(svg, [(bx, y1 + h1), (bx, y2)])
    oy = y2 + 48
    arrow(svg, [(col(2), oy), (col(0) + span2, oy)])
    edge_label(svg, (col(0) + span2 + col(2)) / 2, oy - 10, "zones")
    arrow(svg, [(col(2) + CW, oy), (col(3), oy)])
    edge_label(svg, (col(2) + CW + col(3)) / 2, oy - 10, "reads")
    arrow(svg, [(col(4), oy), (col(3) + CW, oy)])
    edge_label(svg, (col(3) + CW + col(4)) / 2, oy - 10, "boxes")
    fy = y2 + 104
    arrow(svg, [(col(3) + CW, fy), (col(4), fy)], color=t["accent"])
    edge_label(svg, (col(3) + CW + col(4)) / 2, fy - 10, "fixes", color=t["accent_fg"])

    # Row 3: render, right to left under the box tracks.
    y3, h3 = 820, 184
    section(svg, col(1), y3 - 22, "RENDER", "streams behind the analysis; same bytes as analyze + render")
    node(
        svg,
        col(4),
        y3,
        CW,
        h3,
        "blurrd-video",
        "Render decode",
        ["trails the analysis, waits", "for final boxes; PTS check"],
        out="frames + boxes",
    )
    node(
        svg,
        col(3),
        y3,
        CW,
        h3,
        "blurrd-blur",
        "Blur inside boxes",
        ["`CpuBlur` on YUV 4:2:0,", "only inside padded boxes"],
        out="blurred frames",
    )
    node(
        svg,
        col(2),
        y3,
        CW,
        h3,
        "blurrd-video",
        "Encode",
        ["NVENC (H.264: 2 sessions),", "AMF, Quick Sync or MF"],
        out="packets, source GOPs",
    )
    node(
        svg,
        col(1),
        y3,
        CW,
        h3,
        "blurrd-video",
        "Mux",
        ["audio, subtitles copied;", "recorder metadata removed"],
        out="<name>.tmp.<ext>",
    )
    node(
        svg,
        col(0),
        y3,
        CW,
        h3,
        "output",
        "Export",
        ["`<name>.mp4` once verified,", "else `<name>_UNVERIFIED.mp4`"],
        out="size, fps, PTS kept",
        accent=True,
    )
    ry = y3 + 48
    for i in range(4, 0, -1):
        arrow(svg, [(col(i), ry), (col(i - 1) + CW, ry)])
    mid23 = (y2 + h2 + y3) / 2
    arrow(svg, [(bx, y2 + h2), (bx, y3)])
    edge_label(svg, bx + 12, mid23 + 5, "box store", anchor="start")
    sx = col(3) + 64
    ex = col(0) + CW / 2
    gy = mid23 - 8
    arrow(svg, [(sx, y2 + h2), (sx, gy), (ex, gy), (ex, y3)], color=t["accent"])
    edge_label(
        svg,
        (col(1) + col(2) + CW) / 2,
        gy + 5,
        "clean: rename · fixes: re-encode only GOPs with changed boxes",
        color=t["accent_fg"],
        pill=True,
    )

    # Footer: measured speed, hardware and the privacy note.
    sy = 1070
    svg.add(f'<line x1="{X0}" y1="{sy - 40}" x2="{W - X0}" y2="{sy - 40}" stroke="{t["rule"]}"/>')
    stats = [
        ("≈ 10 s", "60 s 1080p60: analysis, OCR, export"),
        ("800–890 fps", "analysis without OCR"),
        ("450–500 fps", "analysis with OCR"),
        ("830–870 fps", "H.264 render, 2 NVENC sessions"),
    ]
    for i, (num, what) in enumerate(stats):
        svg.text(num, col(i), sy, 25, "s", t["fg"])
        svg.text(what, col(i), sy + 27, 15.5, "r", t["sub"], max_w=CW + GAP - 12)
    svg.text("RTX 4080 SUPER, Ryzen 7 7800X3D", col(4), sy - 6, 15.5, "m", t["fg"], max_w=CW)
    svg.text("release build, 2026-10-17", col(4), sy + 15, 15.5, "r", t["sub"])
    svg.text("Everything runs locally.", col(4), sy + 36, 15.5, "r", t["sub"], max_w=CW)

    height = sy + 50
    head = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" viewBox="0 0 {W} {height}" '
        f'role="img" aria-labelledby="title desc">'
        f'<title id="title">How blurrd works</title>'
        f'<desc id="desc">{DESC}</desc><defs>{"".join(svg.glyphs.values())}</defs>'
    )
    return head + "".join(svg.parts) + "</svg>\n", svg.overflow


DESC = (
    "Architecture of blurrd. The desktop app and the command-line tool call the same engine jobs. "
    "Analysis: blurrd-video probes the recording and builds the timestamp table, blurrd-detect "
    "calibrates the HUD layout from sampled keyframes, one decoder (NVDEC, D3D11VA or software) "
    "feeds two to six crop and detect workers and one thread of temporal models, which turn the "
    "detections into box tracks. Verification: up to four OCR readers (PP-OCRv5 on ONNX Runtime "
    "with DirectML or the CPU) read the protected zones during the analysis; afterwards uncovered "
    "name parts are blurred in memory and read again, and readable names get OcrFix boxes, up to "
    "three rounds. Render: a second decoder trails the analysis, blurrd-blur blurs the YUV planes "
    "inside the boxes, NVENC, AMF, Quick Sync or Media Foundation encode, the muxer copies audio, "
    "and the export keeps the source name once verified or is marked UNVERIFIED. Measured on an "
    "RTX 4080 SUPER: about 10 seconds for a 60-second 1080p60 recording. Everything runs locally."
)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--fonts", type=Path, required=True, help="folder with the Inter and JetBrains Mono TTFs")
    args = ap.parse_args()
    fonts = {k: Font.load(args.fonts / name) for k, name in FONT_FILES.items()}
    for name in THEMES:
        svg, overflow = build(name, fonts)
        for o in overflow:
            print("overflow:", o)
        out = OUT / f"how-it-works-{name}.svg"
        out.write_text(svg, encoding="utf-8", newline="\n")
        print(out, f"{len(svg.encode()) / 1024:.0f} KB")


if __name__ == "__main__":
    main()
