#!/usr/bin/env python3
"""Generate the figures for docs/island-exactness-explained.md.

Reads the real artifacts produced by scripts/jm_glue_encode.sh runs in /tmp and
writes PNG frame evidence plus SVG macroblock-grid diagrams into
docs/images/island-exactness/.

Usage:
    python3 scripts/make_exactness_figures.py
"""

import json
import os
import struct
import sys
import zlib
from pathlib import Path

W, H = 1280, 720
YS = W * H
CW, CH = W // 2, H // 2
FS = YS * 3 // 2
COLS, ROWS = W // 16, H // 16
MBS = COLS * ROWS
NFRAMES = 8

OUT = Path(__file__).resolve().parents[1] / "docs" / "images" / "island-exactness"

# Palette matched to docs/presentations/eva-improvements.
BG = "#070b14"
CARD = "#0d1424"
GRID = "#1b2540"
MUTED = "#94a3b8"
FG = "#e2e8f5"
ACCENT = "#22d3ee"
OK = "#10b981"
WARN = "#f59e0b"
BAD = "#ef4444"
PAINT = "#a855f7"

PAINT_MBX = range(30, 35)
PAINT_MBY = range(12, 17)


# ---------------------------------------------------------------- PNG writing


def write_png(path, width, height, rgb_rows):
    """Write 8-bit RGB PNG. rgb_rows is a list of bytes objects, 3*width each."""
    raw = b"".join(b"\x00" + row for row in rgb_rows)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    path.write_bytes(png)


def clamp(v):
    return 0 if v < 0 else (255 if v > 255 else v)


def yuv_frame_to_rgb_rows(buf, frame, x0=0, y0=0, w=W, h=H, scale=1):
    """BT.601 limited-range YUV420p → RGB rows, optional crop and integer upscale."""
    base = frame * FS
    rows = []
    for yy in range(h):
        sy = y0 + yy
        row = bytearray()
        for xx in range(w):
            sx = x0 + xx
            y = buf[base + sy * W + sx]
            u = buf[base + YS + (sy // 2) * CW + (sx // 2)]
            v = buf[base + YS + CW * CH + (sy // 2) * CW + (sx // 2)]
            c = (y - 16) * 298
            d = u - 128
            e = v - 128
            r = clamp((c + 409 * e + 128) >> 8)
            g = clamp((c - 100 * d - 208 * e + 128) >> 8)
            b = clamp((c + 516 * d + 128) >> 8)
            px = bytes((r, g, b))
            row += px * scale
        for _ in range(scale):
            rows.append(bytes(row))
    return rows


def overlay_grid(rows, width, height, step=16, color=(90, 110, 150), every=1):
    """Draw a macroblock grid onto RGB rows in place."""
    out = [bytearray(r) for r in rows]
    for y in range(height):
        if (y % step) == 0:
            for x in range(width):
                i = x * 3
                out[y][i : i + 3] = bytes(color)
        else:
            for x in range(0, width, step):
                i = x * 3
                out[y][i : i + 3] = bytes(color)
    return [bytes(r) for r in out]


def outline_box(rows, width, height, x0, y0, x1, y1, color, thickness=3):
    out = [bytearray(r) for r in rows]
    for t in range(thickness):
        for x in range(max(0, x0 - t), min(width, x1 + t)):
            for y in (y0 - t, y1 + t - 1):
                if 0 <= y < height:
                    out[y][x * 3 : x * 3 + 3] = bytes(color)
        for y in range(max(0, y0 - t), min(height, y1 + t)):
            for x in (x0 - t, x1 + t - 1):
                if 0 <= x < width:
                    out[y][x * 3 : x * 3 + 3] = bytes(color)
    return [bytes(r) for r in out]


# ---------------------------------------------------------------- data loading


def load_yuv(path):
    data = Path(path).read_bytes()
    if len(data) != FS * NFRAMES:
        raise SystemExit(f"{path}: expected {FS*NFRAMES} bytes, got {len(data)}")
    return data


def load_ipcm(path):
    s = set()
    for line in Path(path).read_text().splitlines():
        p = line.split()
        if len(p) >= 3:
            s.add((int(p[0]), int(p[1]), int(p[2])))
    return s


def mb_eq(a, b, f, mx, my):
    base = f * FS
    y0, x0 = my * 16, mx * 16
    for y in range(16):
        i = base + (y0 + y) * W + x0
        if a[i : i + 16] != b[i : i + 16]:
            return False
    for p in (0, 1):
        off = base + YS + p * CW * CH
        for y in range(8):
            i = off + (my * 8 + y) * CW + mx * 8
            if a[i : i + 8] != b[i : i + 8]:
                return False
    return True


def classify(dec, signed, ipcm, paint):
    """Per-(frame,mx,my) label: paint / raw_same / raw_diff / ring / same."""
    out = {}
    for f in range(NFRAMES):
        for my in range(ROWS):
            for mx in range(COLS):
                key = (f, mx, my)
                same = mb_eq(dec, signed, f, mx, my)
                if key in paint:
                    out[key] = "paint"
                elif key in ipcm:
                    out[key] = "raw_same" if same else "raw_diff"
                else:
                    out[key] = "same" if same else "ring"
    return out


# ---------------------------------------------------------------- SVG helpers


def svg_open(width, height, title=""):
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" font-family="Inter, Helvetica, Arial, sans-serif">',
        f'<rect width="{width}" height="{height}" fill="{BG}"/>',
    ]


def txt(x, y, s, size=13, fill=FG, anchor="start", weight="400", mono=False, opacity=1.0):
    fam = ' font-family="ui-monospace, SFMono-Regular, Menlo, monospace"' if mono else ""
    return (
        f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
        f'font-weight="{weight}" opacity="{opacity}"{fam}>{s}</text>'
    )


def legend(x, y, items, size=12, gap=None):
    """Swatch + label row, spaced by measured label width so nothing overlaps."""
    out = []
    cx = x
    for color, label in items:
        out.append(f'<rect x="{cx}" y="{y-9}" width="11" height="11" rx="2" fill="{color}"/>')
        out.append(txt(cx + 17, y, label, size=size, fill=MUTED))
        cx += 17 + int(len(label) * size * 0.56) + 34
    return out


def mb_grid_svg(labels, frame, x, y, cell=7.0, stroke=0.35, colors=None):
    """One frame's 80x45 macroblock grid."""
    colors = colors or {
        "same": "#16233c",
        "raw_same": ACCENT,
        "raw_diff": WARN,
        "ring": BAD,
        "paint": PAINT,
    }
    out = [
        f'<rect x="{x-1}" y="{y-1}" width="{COLS*cell+2}" height="{ROWS*cell+2}" '
        f'fill="none" stroke="{GRID}" stroke-width="1"/>'
    ]
    # Batch same-colour runs per row to keep the SVG small.
    for my in range(ROWS):
        mx = 0
        while mx < COLS:
            lab = labels.get((frame, mx, my), "same")
            run = 1
            while mx + run < COLS and labels.get((frame, mx + run, my), "same") == lab:
                run += 1
            fill = colors[lab]
            out.append(
                f'<rect x="{x+mx*cell:.2f}" y="{y+my*cell:.2f}" '
                f'width="{run*cell:.2f}" height="{cell:.2f}" fill="{fill}" '
                f'stroke="{GRID}" stroke-width="{stroke}"/>'
            )
            mx += run
    return out


# ---------------------------------------------------------------- figures


def fig_frame_grid(signed):
    """Real frame 7 downscaled with a macroblock grid and the paint island marked."""
    scale = 1
    rows = yuv_frame_to_rgb_rows(signed, 7, scale=scale)
    rows = overlay_grid(rows, W, H, step=16, color=(70, 90, 130))
    rows = outline_box(rows, W, H, 30 * 16, 12 * 16, 35 * 16, 17 * 16, (168, 85, 247), 4)
    write_png(OUT / "fig1-frame-macroblocks.png", W, H, rows)


def fig_before_after(signed, published):
    """Side-by-side crop: camera picture vs published picture around the edit."""
    x0, y0 = 30 * 16 - 96, 12 * 16 - 80
    cw, chh = 80 * 4, 80 * 4
    a = yuv_frame_to_rgb_rows(signed, 7, x0, y0, cw, chh)
    b = yuv_frame_to_rgb_rows(published, 7, x0, y0, cw, chh)
    gapw = 14
    gap = bytes((13, 20, 36)) * gapw
    rows = [a[i] + gap + b[i] for i in range(chh)]
    write_png(OUT / "fig2-before-after.png", cw * 2 + gapw, chh, rows)


def fig_diff_map(signed, published, name):
    """Absolute luma difference on frame 7, boosted, as a heat image."""
    base = 7 * FS
    rows = []
    for y in range(H):
        row = bytearray()
        for x in range(W):
            d = abs(published[base + y * W + x] - signed[base + y * W + x])
            if d == 0:
                row += bytes((10, 16, 28))
            else:
                v = min(255, 40 + d * 24)
                row += bytes((v, max(0, 120 - d * 8), 40))
        rows.append(bytes(row))
    write_png(OUT / name, W, H, rows)


def fig_class_grid(labels_a, stats_a, labels_b, stats_b):
    """Frame 7 macroblock classification: deblocking on vs deblocking off.

    Each panel carries its own colour key, so there is no separate legend to
    cross-reference while reading the grid.
    """
    cell = 7.0
    gw, gh = COLS * cell, ROWS * cell
    width = int(gw * 2 + 170)
    height = int(gh + 300)
    s = svg_open(width, height)
    s.append(
        txt(40, 46, "Every macroblock of the edited frame, compared with the camera’s picture",
            size=20, weight="600")
    )
    s.append(
        txt(
            40,
            72,
            "One cell = one 16×16 macroblock. 80 across × 45 down = 3,600 cells. "
            "Colour says what happened to that block.",
            size=13,
            fill=MUTED,
        )
    )

    for i, (labels, stats, title) in enumerate(
        ((labels_a, stats_a, "Camera clip WITH the smoothing filter"),
         (labels_b, stats_b, "Camera clip WITHOUT the smoothing filter"))
    ):
        x = 40 + i * (gw + 90)
        same_total = stats["same"] + stats["raw_same"]
        s.append(txt(x, 112, title, size=15.5, weight="600", fill=FG))
        s.append(
            txt(
                x,
                134,
                f"{same_total:,} of 3,600 blocks are bit-identical to the camera",
                size=13,
                fill=OK if stats["ring"] == 0 and stats["raw_diff"] == 0 else MUTED,
            )
        )
        s += mb_grid_svg(labels, 7, x, 150, cell=cell)

        # Per-panel key with counts, one row each: easy to read next to the grid.
        key = [
            ("#16233c", "untouched, left exactly as the camera wrote it", stats["same"]),
            (ACCENT, "stored raw, decodes to the camera’s pixels", stats["raw_same"]),
            (WARN, "stored raw, still differs from the camera", stats["raw_diff"]),
            (BAD, "smoothing-filter ring, differs by ≤3 of 255", stats["ring"]),
            (PAINT, "the edit itself", stats["paint"]),
        ]
        ky = 150 + gh + 34
        for j, (color, label, count) in enumerate(key):
            yy = ky + j * 22
            s.append(f'<rect x="{x}" y="{yy-10}" width="12" height="12" rx="2" fill="{color}"/>')
            s.append(txt(x + 22, yy, f"{count:,}", size=12.5, fill=FG, mono=True))
            s.append(txt(x + 74, yy, label, size=12.5, fill=MUTED))
    s.append("</svg>")
    (OUT / "fig3-frame7-classification.svg").write_text("\n".join(s))


def fig_raw_map(labels, ipcm, frame=7):
    """Which blocks were stored raw, on its own, with the edit marked."""
    cell = 11.0
    gw, gh = COLS * cell, ROWS * cell
    width = int(gw + 80)
    height = int(gh + 220)
    s = svg_open(width, height)
    s.append(txt(40, 46, "Which blocks were stored raw (I_PCM)", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "Edit on the last frame, camera encoded with no smoothing filter. Every\u00a0other "
            "block keeps the camera’s own bits.",
            size=13,
            fill=MUTED,
        )
    )
    colors = {
        "same": "#16233c",
        "raw_same": ACCENT,
        "raw_diff": WARN,
        "ring": BAD,
        "paint": PAINT,
    }
    s += mb_grid_svg(labels, frame, 40, 100, cell=cell, colors=colors)
    n_raw = sum(1 for k in ipcm if k[0] == frame)
    ky = 100 + gh + 36
    for j, (color, label, count) in enumerate(
        [
            (PAINT, "the edit: painted by the editor", 25),
            (ACCENT, "raw because their prediction changed (the cascade)", n_raw - 25),
            ("#16233c", "untouched: the camera’s original bits, re-emitted", 3600 - n_raw),
        ]
    ):
        yy = ky + j * 24
        s.append(f'<rect x="40" y="{yy-11}" width="13" height="13" rx="2" fill="{color}"/>')
        s.append(txt(64, yy, f"{count:,}", size=13, fill=FG, mono=True))
        s.append(txt(124, yy, label, size=13, fill=MUTED))
    s.append(
        txt(
            40,
            ky + 3 * 24 + 16,
            "The cascade hangs below and right of the edit: prediction flows that way, "
            "because a block reads its left and upper neighbours.",
            size=12.5,
            fill=MUTED,
        )
    )
    s.append("</svg>")
    (OUT / "fig11-raw-blocks.svg").write_text("\n".join(s))


def fig_time_stack(series, name, title, subtitle, note):
    """Stacked per-frame macroblock grids, drawn as a receding stack (time axis)."""
    cell = 3.4
    gw, gh = COLS * cell, ROWS * cell
    dx, dy = 40, 30
    stack_w = gw + dx * (NFRAMES - 1)
    label_x = 40 + stack_w + 40
    row_h = 46
    width = int(label_x + 330)
    height = int(max(gh + dy * (NFRAMES - 1), row_h * NFRAMES) + 250)

    s = svg_open(width, height)
    s.append(txt(40, 46, title, size=20, weight="600"))
    s.append(txt(40, 72, subtitle, size=13, fill=MUTED))
    top = 118

    # Draw back-to-front so nearer (later) frames overlap earlier ones.
    for f in range(NFRAMES - 1, -1, -1):
        x = 40 + dx * (NFRAMES - 1 - f)
        y = top + dy * (NFRAMES - 1 - f)
        labels, raw, diff = series[f]
        s.append(
            f'<rect x="{x-5}" y="{y-5}" width="{gw+10}" height="{gh+10}" rx="4" '
            f'fill="{CARD}" stroke="{GRID}" stroke-width="1"/>'
        )
        s += mb_grid_svg(labels, f, x, y, cell=cell, stroke=0.0)

    # One tidy label row per frame, newest at the bottom, with a leader line.
    for f in range(NFRAMES):
        ly = top + 16 + (NFRAMES - 1 - f) * row_h
        gx = 40 + dx * (NFRAMES - 1 - f) + gw
        gy = top + dy * (NFRAMES - 1 - f) + gh / 2
        s.append(
            f'<path d="M {gx+6} {gy:.1f} L {label_x-12} {ly-4}" stroke="{GRID}" '
            f'stroke-width="1"/>'
        )
        labels, raw, diff = series[f]
        s.append(txt(label_x, ly, f"frame {f}", size=13, weight="600", fill=FG, mono=True))
        s.append(
            txt(
                label_x + 78,
                ly,
                f"{raw:,} raw blocks" if raw else "byte-identical copy",
                size=12.5,
                fill=ACCENT if raw else OK,
                mono=True,
            )
        )
        s.append(
            txt(
                label_x + 78,
                ly + 18,
                f"{diff:,} unlike camera" if diff else "none unlike camera",
                size=12.5,
                fill=BAD if diff else OK,
                mono=True,
            )
        )

    s.append(txt(40, height - 82, "time →", size=13, fill=MUTED, weight="600"))
    s.append(txt(40, height - 58, note, size=12.5, fill=MUTED))
    s += legend(
        40,
        height - 26,
        [
            ("#16233c", "untouched, identical"),
            (ACCENT, "raw block, identical"),
            (WARN, "raw block, differs"),
            (BAD, "differs from camera"),
            (PAINT, "the edit"),
        ],
    )
    s.append("</svg>")
    (OUT / name).write_text("\n".join(s))


def fig_samples():
    """Why an ordinary residual cannot hit an exact target, and what raw does."""
    width, height = 1180, 520
    s = svg_open(width, height)
    s.append(txt(40, 46, "Why some blocks have to be stored raw", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "A decoder rebuilds every macroblock as prediction + residual. "
            "The residual travels in coarse steps.",
            size=13,
            fill=MUTED,
        )
    )

    def box(x, y, w, h, fill, stroke, label, sub, value=None):
        o = [
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{fill}" '
            f'stroke="{stroke}" stroke-width="1.4"/>',
            txt(x + w / 2, y + 30, label, size=15, anchor="middle", weight="600", fill=FG),
        ]
        if sub:
            o.append(txt(x + w / 2, y + 52, sub, size=12, anchor="middle", fill=MUTED))
        if value:
            o.append(txt(x + w / 2, y + 78, value, size=13, anchor="middle", fill=ACCENT, mono=True))
        return o

    y0 = 116
    s += box(40, y0, 210, 100, CARD, GRID, "prediction", "from neighbours or", "an earlier frame")
    s.append(txt(272, y0 + 58, "+", size=26, fill=MUTED, anchor="middle"))
    s += box(296, y0, 210, 100, CARD, GRID, "residual", "quantised, step ≈ 16", "at quality QP 28")
    s.append(txt(528, y0 + 58, "=", size=26, fill=MUTED, anchor="middle"))
    s += box(552, y0, 210, 100, CARD, ACCENT, "samples", "what you see", None)

    s.append(
        txt(
            40,
            y0 + 152,
            "The editor changed one neighbour. The prediction is now different, "
            "so the camera’s stored residual lands somewhere else.",
            size=13.5,
            fill=FG,
        )
    )
    s.append(
        txt(
            40,
            y0 + 176,
            "Correcting it needs a residual of, say, 3 — smaller than one step, so it is "
            "stored as 0. The block cannot be made exact this way.",
            size=13.5,
            fill=MUTED,
        )
    )

    y1 = y0 + 216
    s += box(40, y1, 466, 108, "#131d33", PAINT, "I_PCM: store the 384 samples themselves", "no prediction, no residual, no rounding", "256 luma + 64 U + 64 V bytes")
    s.append(
        f'<path d="M 522 {y1+54} L 566 {y1+54}" stroke="{MUTED}" stroke-width="1.6" '
        f'marker-end="url(#a)"/>'
    )
    s.append(
        f'<defs><marker id="a" markerWidth="8" markerHeight="8" refX="6" refY="3" orient="auto">'
        f'<path d="M0,0 L6,3 L0,6 z" fill="{MUTED}"/></marker></defs>'
    )
    s += box(578, y1, 300, 108, CARD, OK, "exactly the camera’s pixels", "bit-for-bit, every time", None)
    s.append(
        txt(
            900,
            y1 + 44,
            "cost: 384 bytes",
            size=13,
            fill=WARN,
            mono=True,
        )
    )
    s.append(
        txt(
            900,
            y1 + 64,
            "vs ≈ 30 bytes normally",
            size=13,
            fill=MUTED,
            mono=True,
        )
    )
    s.append("</svg>")
    (OUT / "fig4-prediction-residual.svg").write_text("\n".join(s))


def fig_cascade_mechanism():
    """How one rewritten block forces its neighbours."""
    width, height = 1180, 470
    s = svg_open(width, height)
    s.append(txt(40, 46, "How one rewritten block forces others: the cascade", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "Three ways a neighbour stops decoding to the camera’s pixels once a block is stored raw.",
            size=13,
            fill=MUTED,
        )
    )

    cell = 30
    def mini(x, y, marks, arrows=()):
        o = []
        for gy in range(4):
            for gx in range(4):
                fill = marks.get((gx, gy), "#16233c")
                o.append(
                    f'<rect x="{x+gx*cell}" y="{y+gy*cell}" width="{cell}" height="{cell}" '
                    f'fill="{fill}" stroke="{GRID}" stroke-width="1"/>'
                )
        for (ax, ay, bx, by) in arrows:
            o.append(
                f'<path d="M {x+ax*cell+cell/2} {y+ay*cell+cell/2} '
                f'L {x+bx*cell+cell/2} {y+by*cell+cell/2}" stroke="{ACCENT}" '
                f'stroke-width="2" marker-end="url(#c)"/>'
            )
        return o

    s.append(
        f'<defs><marker id="c" markerWidth="9" markerHeight="9" refX="7" refY="3.2" orient="auto">'
        f'<path d="M0,0 L7,3.2 L0,6.4 z" fill="{ACCENT}"/></marker></defs>'
    )

    panels = [
        (
            "1 · spatial prediction",
            "A block copies pixels from the edge of the block to its left and above. "
            "Those pixels changed, so its own pixels change.",
            {(1, 1): PAINT, (2, 1): WARN, (1, 2): WARN},
            ((1, 1, 2, 1), (1, 1, 1, 2)),
        ),
        (
            "2 · motion vector prediction",
            "A raw block has no motion vector. Its neighbour’s vector is guessed from it, "
            "so that guess moves and the stored correction misses.",
            {(1, 1): PAINT, (2, 1): WARN},
            ((1, 1, 2, 1),),
        ),
        (
            "3 · motion compensation",
            "A block on the next frame copies a patch from this picture. "
            "If that patch moved, the copy lands on different pixels.",
            {(1, 1): PAINT, (2, 2): WARN},
            ((1, 1, 2, 2),),
        ),
    ]
    for i, (title, body, marks, arrows) in enumerate(panels):
        x = 40 + i * 380
        s.append(
            f'<rect x="{x-14}" y="100" width="352" height="330" rx="10" fill="{CARD}" '
            f'stroke="{GRID}" stroke-width="1"/>'
        )
        s.append(txt(x, 130, title, size=15, weight="600", fill=ACCENT))
        s += mini(x + 88, 152, marks, arrows)
        # wrap body text
        words = body.split()
        line, lines = "", []
        for wd in words:
            if len(line) + len(wd) > 44:
                lines.append(line)
                line = wd
            else:
                line = (line + " " + wd).strip()
        lines.append(line)
        for j, ln in enumerate(lines):
            s.append(txt(x, 306 + j * 20, ln, size=12.5, fill=MUTED))

    s += legend(40, height - 14, [(PAINT, "block stored raw"), (WARN, "forced to be rewritten too")], gap=210)
    s.append("</svg>")
    (OUT / "fig5-cascade-mechanism.svg").write_text("\n".join(s))


def fig_deblock_ring():
    """Why the smoothing filter leaves a ring, and why turning it off removes it."""
    width, height = 1180, 430
    s = svg_open(width, height)
    s.append(txt(40, 46, "The smoothing filter is what stops exactness", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "A decoder smooths across every block border. Inside a raw block it is skipped, "
            "and it is skipped on that block’s borders too.",
            size=13,
            fill=MUTED,
        )
    )

    def strip(x, y, label, left_fill, right_fill, filt, note, note_col):
        o = [
            f'<rect x="{x-14}" y="{y-34}" width="520" height="210" rx="10" fill="{CARD}" '
            f'stroke="{GRID}" stroke-width="1"/>',
            txt(x, y - 10, label, size=15, weight="600", fill=ACCENT),
        ]
        cell = 26
        for i in range(8):
            fill = left_fill if i < 4 else right_fill
            o.append(
                f'<rect x="{x+i*cell}" y="{y+16}" width="{cell}" height="{cell*2}" '
                f'fill="{fill}" stroke="{GRID}" stroke-width="1"/>'
            )
        bx = x + 4 * cell
        o.append(
            f'<line x1="{bx}" y1="{y+10}" x2="{bx}" y2="{y+16+cell*2+8}" '
            f'stroke="{FG}" stroke-width="2.2"/>'
        )
        o.append(txt(bx, y + 16 + cell * 2 + 26, "block border", size=11.5, fill=MUTED, anchor="middle"))
        if filt:
            o.append(
                f'<rect x="{bx-3*cell}" y="{y+16}" width="{6*cell}" height="{cell*2}" '
                f'fill="{ACCENT}" opacity="0.16"/>'
            )
            o.append(txt(bx + 3.4 * cell, y + 40, "smoothed", size=12, fill=ACCENT))
        else:
            o.append(txt(bx + 3.4 * cell, y + 40, "not smoothed", size=12, fill=BAD))
        o.append(txt(x, y + 16 + cell * 2 + 58, note, size=12.5, fill=note_col))
        return o

    s += strip(
        40,
        150,
        "Camera file: two ordinary blocks",
        "#1d2a47",
        "#1d2a47",
        True,
        "This smoothing is part of the signed picture.",
        MUTED,
    )
    s += strip(
        620,
        150,
        "Published file: raw block on the left",
        PAINT,
        "#1d2a47",
        False,
        "The neighbour keeps a 2-pixel strip the camera smoothed: off by up to 3 of 255.",
        BAD,
    )
    s.append(
        txt(
            40,
            height - 28,
            "Making that neighbour raw as well just moves the border outward, and a new ring "
            "appears. Measured: 636 → 149 → 215 → 103 → 106 → 92 blocks over six rounds, never zero.",
            size=13,
            fill=FG,
        )
    )
    s.append("</svg>")
    (OUT / "fig6-deblocking-ring.svg").write_text("\n".join(s))


def fig_motion_vector():
    """What a motion vector is, and what 'predicted' means."""
    width, height = 1180, 620
    s = svg_open(width, height)
    s.append(txt(40, 46, "Motion vectors, and why they are guessed", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "A block on a new frame is usually described as “the same thing, moved”.",
            size=13,
            fill=MUTED,
        )
    )

    cell = 26
    def frame_box(x, y, label, cols=9, rows=6):
        o = [
            f'<rect x="{x-8}" y="{y-8}" width="{cols*cell+16}" height="{rows*cell+16}" rx="6" '
            f'fill="{CARD}" stroke="{GRID}" stroke-width="1"/>',
            txt(x, y - 18, label, size=13, fill=MUTED, mono=True),
        ]
        for gy in range(rows):
            for gx in range(cols):
                o.append(
                    f'<rect x="{x+gx*cell}" y="{y+gy*cell}" width="{cell}" height="{cell}" '
                    f'fill="#16233c" stroke="{GRID}" stroke-width="0.6"/>'
                )
        return o

    # Panel 1: the vector itself.
    y0 = 118
    s += frame_box(60, y0, "previous frame (already decoded)")
    s += frame_box(400, y0, "frame being decoded")
    # Source patch sits right and below; the block being decoded is up and left.
    s.append(
        f'<rect x="{60+5*cell}" y="{y0+3*cell}" width="{cell}" height="{cell}" '
        f'fill="{OK}" opacity="0.85"/>'
    )
    s.append(txt(60 + 5.5 * cell, y0 + 3 * cell - 6, "this patch", size=11, anchor="middle", fill=OK))
    s.append(
        f'<rect x="{400+2*cell}" y="{y0+1*cell}" width="{cell}" height="{cell}" '
        f'fill="{ACCENT}" opacity="0.85"/>'
    )
    s.append(
        txt(400 + 2.5 * cell, y0 + 1 * cell - 6, "goes here", size=11, anchor="middle", fill=ACCENT)
    )
    s.append(
        f'<path d="M {60+5.5*cell} {y0+3.5*cell} C {300} {y0+160}, {330} {y0+30}, '
        f'{400+2.4*cell} {y0+1.7*cell}" stroke="{ACCENT}" stroke-width="2" fill="none" '
        f'marker-end="url(#mv)"/>'
    )
    s.append(
        f'<defs><marker id="mv" markerWidth="9" markerHeight="9" refX="7" refY="3.2" '
        f'orient="auto"><path d="M0,0 L7,3.2 L0,6.4 z" fill="{ACCENT}"/></marker></defs>'
    )
    s.append(
        txt(
            660,
            y0 + 40,
            "“For this block, copy the patch that sits",
            size=13.5,
            fill=FG,
        )
    )
    s.append(
        txt(660, y0 + 60, "3 blocks right and 2 blocks down in the", size=13.5, fill=FG)
    )
    s.append(txt(660, y0 + 80, "previous frame.”", size=13.5, fill=FG))
    s.append(
        txt(
            660,
            y0 + 110,
            "That offset is the motion vector: two numbers,",
            size=12.5,
            fill=MUTED,
        )
    )
    s.append(txt(660, y0 + 128, "here (+48, +32) in pixels.", size=12.5, fill=ACCENT, mono=True))
    s.append(
        txt(
            660,
            y0 + 156,
            "Any leftover difference is the residual.",
            size=12.5,
            fill=MUTED,
        )
    )

    # Panel 2: prediction of the vector from neighbours.
    y1 = 360
    s.append(
        f'<rect x="46" y="{y1-34}" width="1088" height="230" rx="10" fill="{CARD}" '
        f'stroke="{GRID}" stroke-width="1"/>'
    )
    s.append(txt(66, y1 - 8, "The file does not store the vector. It stores the surprise.", size=15.5, weight="600", fill=ACCENT))

    bx, by = 110, y1 + 22
    # A = left, B = above, C = above-right of the block being decoded.
    letters = {(0, 1): "A", (1, 0): "B", (2, 0): "C"}
    for gx in range(4):
        for gy in range(2):
            fill = "#16233c"
            if (gx, gy) == (1, 1):
                fill = ACCENT
            elif (gx, gy) in letters:
                fill = "#2b3c63"
            s.append(
                f'<rect x="{bx+gx*cell*1.6}" y="{by+gy*cell*1.6}" width="{cell*1.6}" '
                f'height="{cell*1.6}" fill="{fill}" stroke="{GRID}" stroke-width="0.8"/>'
            )
            if (gx, gy) in letters:
                s.append(
                    txt(
                        bx + (gx + 0.5) * cell * 1.6,
                        by + (gy + 0.62) * cell * 1.6,
                        letters[(gx, gy)],
                        size=15,
                        anchor="middle",
                        fill=FG,
                        weight="600",
                        mono=True,
                    )
                )
    s.append(txt(bx + 1.6 * cell * 1.6, by - 10, "already-decoded neighbours", size=11.5, fill=MUTED))
    s.append(txt(bx + 1.0 * cell * 1.6, by + 2.5 * cell * 1.6, "this block", size=11.5, fill=ACCENT))

    tx = 470
    lines = [
        ("A is the block to the left, B above, C above-right.", MUTED, False),
        ("A (+46, +30)    B (+50, +34)    C (+48, +30)", FG, True),
        ("The decoder takes the middle value of each column:", MUTED, False),
        ("predicted vector = (+48, +30)", ACCENT, True),
        ("The file stores only the difference from that guess:", MUTED, False),
        ("stored difference = (0, +2)   →   real vector (+48, +32)", OK, True),
    ]
    for j, (ln, col, mono) in enumerate(lines):
        s.append(txt(tx, by + 6 + j * 26, ln, size=12.8, fill=col, mono=mono))

    s.append(
        txt(
            66,
            y1 + 176,
            "This is why a raw block hurts its neighbour. A raw block carries no vector at all, "
            "so it drops out of that guess, the guess moves,",
            size=12.8,
            fill=MUTED,
        )
    )
    s.append(
        txt(
            66,
            y1 + 196,
            "and the neighbour’s stored difference of (0, +2) now points somewhere else entirely.",
            size=12.8,
            fill=BAD,
        )
    )
    s.append("</svg>")
    (OUT / "fig10-motion-vector.svg").write_text("\n".join(s))


def fig_cabac_bug():
    """The leftover mvd field and why it desynchronised the two sides."""
    width, height = 1180, 560
    s = svg_open(width, height)
    s.append(txt(40, 46, "The bug: one leftover field, and both sides stop agreeing", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "H.264 compresses each number using the neighbouring numbers as context. "
            "Writer and reader must hold the same context, always.",
            size=13,
            fill=MUTED,
        )
    )

    cell = 62

    def pair(x, y, title, left_val, right_ctx, outcome, outcome_col, border):
        o = [
            f'<rect x="{x-16}" y="{y-40}" width="520" height="290" rx="10" fill="{CARD}" '
            f'stroke="{border}" stroke-width="1.4"/>',
            txt(x, y - 14, title, size=15, weight="600", fill=border),
        ]
        # two adjacent blocks
        o.append(
            f'<rect x="{x}" y="{y+10}" width="{cell*1.7}" height="{cell*1.3}" fill="{PAINT}" '
            f'opacity="0.85" stroke="{GRID}"/>'
        )
        o.append(txt(x + cell * 0.85, y + 42, "raw block", size=12, anchor="middle", fill="#fff", weight="600"))
        o.append(txt(x + cell * 0.85, y + 62, "(I_PCM)", size=11, anchor="middle", fill="#f2e8ff"))
        o.append(
            f'<rect x="{x+cell*1.7}" y="{y+10}" width="{cell*1.7}" height="{cell*1.3}" '
            f'fill="#1d2a47" stroke="{GRID}"/>'
        )
        o.append(txt(x + cell * 2.55, y + 46, "next block", size=12, anchor="middle", fill=FG))

        o.append(txt(x, y + 118, "what each side thinks the raw block’s", size=12.5, fill=MUTED))
        o.append(txt(x, y + 136, "motion-vector difference was:", size=12.5, fill=MUTED))
        o.append(txt(x, y + 162, f"encoder: {left_val}", size=13, fill=FG, mono=True))
        o.append(txt(x, y + 182, f"decoder: {right_ctx}", size=13, fill=FG, mono=True))
        o.append(txt(x, y + 214, outcome, size=13, fill=outcome_col, weight="600"))
        return o

    s += pair(
        60, 140, "Before the fix",
        "(0, +2)  ← stale, from the block it replaced",
        "(0, 0)   ← the standard says a raw block has none",
        "Different context → different bits → the streams part company.",
        BAD, BAD,
    )
    s += pair(
        640, 140, "After the fix",
        "(0, 0)   ← cleared when the block became raw",
        "(0, 0)",
        "Same context → the same bits → every decoder agrees.",
        OK, OK,
    )

    s.append(
        txt(
            40,
            470,
            "What it looked like: ffmpeg and the reference decoder matched on frames 0 and 1, "
            "split at one block on frame 2, and from the next row",
            size=12.8,
            fill=MUTED,
        )
    )
    s.append(
        txt(
            40,
            490,
            "the whole frame was unrelated to the intended picture. The fix is two assignments, "
            "applied at the moment a block is switched to raw.",
            size=12.8,
            fill=MUTED,
        )
    )
    s.append(
        txt(
            40,
            522,
            "memset(currMB->mvd, 0, sizeof(currMB->mvd));      currMB->prev_dqp = 0;",
            size=12.5,
            fill=ACCENT,
            mono=True,
        )
    )
    s.append("</svg>")
    (OUT / "fig12-cabac-context.svg").write_text("\n".join(s))


def fig_deblock_pixels():
    """Pixel-level view of the smoothing filter and the ring, with measured values."""
    width, height = 1180, 660
    s = svg_open(width, height)
    s.append(txt(40, 46, "The smoothing filter, pixel by pixel", size=20, weight="600"))
    s.append(
        txt(
            40,
            72,
            "Real values from row 13 of macroblock (36, 15) on the edited frame. "
            "Its left neighbour is a raw block.",
            size=13,
            fill=MUTED,
        )
    )

    # Measured: raw neighbour last 4 px, then the ring block's first 8 px.
    left_cam = [163, 185, 198, 194]
    ring_cam = [196, 201, 202, 205, 207, 195, 168, 153]
    ring_pub = [198, 202, 202, 205, 207, 195, 168, 153]

    cw_ = 56
    def row(x, y, label, left, right, highlight=(), label_col=FG):
        o = [txt(x, y + 24, label, size=13, fill=label_col, weight="600")]
        x0 = x + 250
        for i, v in enumerate(left):
            o.append(
                f'<rect x="{x0+i*cw_}" y="{y}" width="{cw_}" height="38" fill="{PAINT}" '
                f'opacity="0.5" stroke="{GRID}"/>'
            )
            o.append(txt(x0 + i * cw_ + cw_ / 2, y + 25, str(v), size=13, anchor="middle", fill="#fff", mono=True))
        bx = x0 + len(left) * cw_
        for i, v in enumerate(right):
            fill = BAD if i in highlight else "#1d2a47"
            o.append(
                f'<rect x="{bx+i*cw_}" y="{y}" width="{cw_}" height="38" fill="{fill}" '
                f'stroke="{GRID}"/>'
            )
            o.append(txt(bx + i * cw_ + cw_ / 2, y + 25, str(v), size=13, anchor="middle", fill=FG, mono=True))
        o.append(f'<line x1="{bx}" y1="{y-8}" x2="{bx}" y2="{y+46}" stroke="{FG}" stroke-width="2.5"/>')
        return o

    s.append(txt(290, 122, "← raw block (its last 4 pixels)", size=12, fill=MUTED))
    s.append(txt(540, 122, "the neighbour block (its first 8 pixels) →", size=12, fill=MUTED))

    s += row(60, 134, "camera’s picture", left_cam, ring_cam)
    s.append(
        txt(
            60,
            196,
            "the filter ran here: these values were blended across the border",
            size=12,
            fill=ACCENT,
        )
    )
    s += row(60, 226, "published picture", left_cam, ring_pub, highlight=(0, 1))
    s.append(
        txt(
            60,
            288,
            "the filter was skipped: the first two pixels stayed unblended",
            size=12,
            fill=BAD,
        )
    )
    s += row(60, 318, "difference", [0, 0, 0, 0],
             [p - c for p, c in zip(ring_pub, ring_cam)], highlight=(0, 1))

    # Why it is skipped.
    y2 = 410
    s.append(
        f'<rect x="46" y="{y2-30}" width="1088" height="132" rx="10" fill="{CARD}" '
        f'stroke="{GRID}" stroke-width="1"/>'
    )
    s.append(txt(66, y2 - 4, "Why the filter skips that border", size=15, weight="600", fill=ACCENT))
    s.append(
        txt(
            66,
            y2 + 24,
            "Filter strength is set by the average quality number of the two blocks. "
            "The standard fixes a raw block’s number at 0.",
            size=12.8,
            fill=MUTED,
        )
    )
    s.append(
        txt(
            66,
            y2 + 48,
            "average of 0 and 28  =  14      →      at 14 both filter thresholds are 0      →      "
            "the decoder filters nothing across that border",
            size=12.8,
            fill=FG,
            mono=True,
        )
    )
    s.append(
        txt(
            66,
            y2 + 76,
            "It is skipped on all four borders of the raw block, and on every border inside it.",
            size=12.8,
            fill=MUTED,
        )
    )

    # Scale of the effect.
    y3 = 566
    s.append(txt(40, y3, "How big is the whole effect, measured over the frame", size=15, weight="600"))
    facts = [
        ("54", "macroblocks touched by the ring, out of 3,600"),
        ("349", "luma samples differ, out of 13,824 in those blocks (2.5%)"),
        ("3", "largest difference, on a 0–255 scale"),
        ("2 px", "deepest reach into the neighbour"),
    ]
    for i, (n, label) in enumerate(facts):
        x = 40 + i * 285
        s.append(txt(x, y3 + 34, n, size=22, fill=ACCENT, weight="600", mono=True))
        words = label.split()
        line, lines = "", []
        for wd in words:
            if len(line) + len(wd) > 34:
                lines.append(line)
                line = wd
            else:
                line = (line + " " + wd).strip()
        lines.append(line)
        for j, ln in enumerate(lines):
            s.append(txt(x, y3 + 56 + j * 17, ln, size=12, fill=MUTED))
    s.append("</svg>")
    (OUT / "fig13-deblocking-pixels.svg").write_text("\n".join(s))


def fig_protocol():
    """Signed picture → proof → published file → viewer."""
    width, height = 1180, 430
    s = svg_open(width, height)
    s.append(txt(40, 46, "What is signed, what is proved, what is shipped", size=20, weight="600"))

    def node(x, y, w, h, title, lines, stroke):
        o = [
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="{CARD}" '
            f'stroke="{stroke}" stroke-width="1.5"/>',
            txt(x + 18, y + 30, title, size=14.5, weight="600", fill=FG),
        ]
        for i, ln in enumerate(lines):
            o.append(txt(x + 18, y + 54 + i * 19, ln, size=12, fill=MUTED))
        return o

    def arrow(x1, y1, x2, y2, label):
        o = [
            f'<path d="M {x1} {y1} L {x2} {y2}" stroke="{MUTED}" stroke-width="1.6" '
            f'marker-end="url(#p)"/>'
        ]
        if label:
            o.append(txt((x1 + x2) / 2, y1 - 10, label, size=11.5, fill=ACCENT, anchor="middle"))
        return o

    s.append(
        f'<defs><marker id="p" markerWidth="9" markerHeight="9" refX="7" refY="3.2" orient="auto">'
        f'<path d="M0,0 L7,3.2 L0,6.4 z" fill="{MUTED}"/></marker></defs>'
    )

    s += node(40, 100, 250, 120, "Camera", ["signs the H.264 file", "and the picture it decodes to", "→ R_pix"], ACCENT)
    s += arrow(298, 160, 348, 160, "")
    s += node(356, 100, 250, 120, "Editor", ["paints 25 macroblocks", "on the decoded picture", "→ R_stmt"], PAINT)
    s += arrow(614, 160, 664, 160, "")
    s += node(672, 100, 250, 120, "Proof", ["25 macroblocks only", "opens under both roots", "cascade is not in here"], OK)

    s += node(40, 260, 566, 120, "Publisher: rebuild the H.264",
              ["keeps the camera’s own bits for every untouched block",
               "stores raw samples where the prediction changed (669 blocks)",
               "file 564 KB vs 318 KB original"], WARN)
    s += arrow(614, 320, 664, 320, "")
    s += node(672, 260, 250, 120, "Any player",
              ["decodes to exactly R_stmt",
               "ffmpeg and the reference",
               "decoder agree, byte for byte"], OK)
    s.append("</svg>")
    (OUT / "fig7-protocol.svg").write_text("\n".join(s))


# ---------------------------------------------------------------- main


def main():
    OUT.mkdir(parents=True, exist_ok=True)

    need = {
        "dfoff_src": "/tmp/dfoff-src.yuv",
        "dfoff_pub7": "/tmp/dfoff-exact-f7-ff.yuv",
        "dfoff_ipcm7": "/tmp/dfoff-ipcm-7.txt",
        "dfoff_pub0": "/tmp/dfoff-exact-0-ff.yuv",
        "dfoff_ipcm0": "/tmp/dfoff-ipcm-0.txt",
        "dfon_src": "/tmp/glue-8f-ref1-src.yuv",
        "dfon_pub7": "/tmp/twopass-1-ff.yuv",
        "dfon_ipcm7": "/tmp/twopass-1-ipcm.txt",
        "dfon_pub0": "/tmp/tp0-0-ff.yuv",
        "dfon_ipcm0": "/tmp/tp0-0-ipcm.txt",
    }
    missing = [p for p in need.values() if not os.path.exists(p)]
    if missing:
        print("missing artifacts:", *missing, sep="\n  ")
        return 1

    dfoff_src = load_yuv(need["dfoff_src"])
    dfoff_pub7 = load_yuv(need["dfoff_pub7"])
    dfon_src = load_yuv(need["dfon_src"])
    dfon_pub7 = load_yuv(need["dfon_pub7"])

    paint7 = {(7, mx, my) for my in PAINT_MBY for mx in PAINT_MBX}
    paint0 = {(0, mx, my) for my in PAINT_MBY for mx in PAINT_MBX}

    print("classifying frame 7 (filter on / filter off)…")
    lab_on = classify(dfon_pub7, dfon_src, load_ipcm(need["dfon_ipcm7"]), paint7)
    lab_off = classify(dfoff_pub7, dfoff_src, load_ipcm(need["dfoff_ipcm7"]), paint7)

    def stats(labels, frame):
        out = {"same": 0, "raw_same": 0, "raw_diff": 0, "ring": 0, "paint": 0}
        for my in range(ROWS):
            for mx in range(COLS):
                out[labels[(frame, mx, my)]] += 1
        return out

    st_on, st_off = stats(lab_on, 7), stats(lab_off, 7)
    print("  filter on ", st_on)
    print("  filter off", st_off)

    fig_class_grid(lab_on, st_on, lab_off, st_off)

    print("frame evidence PNGs…")
    fig_frame_grid(dfoff_src)
    fig_before_after(dfoff_src, dfoff_pub7)
    fig_diff_map(dfon_src, dfon_pub7, "fig8-diff-filter-on.png")
    fig_diff_map(dfoff_src, dfoff_pub7, "fig9-diff-filter-off.png")

    print("time stacks…")
    dfon_pub0 = load_yuv(need["dfon_pub0"])
    dfoff_pub0 = load_yuv(need["dfoff_pub0"])
    ip_on0 = load_ipcm(need["dfon_ipcm0"])
    ip_off0 = load_ipcm(need["dfoff_ipcm0"])
    lab_on0 = classify(dfon_pub0, dfon_src, ip_on0, paint0)
    lab_off0 = classify(dfoff_pub0, dfoff_src, ip_off0, paint0)

    def series(labels, ipcm):
        """Per frame: labels, raw-block count, and MBs unlike the camera outside the edit."""
        out = {}
        for f in range(NFRAMES):
            st = stats(labels, f)
            raw = sum(1 for k in ipcm if k[0] == f)
            out[f] = (labels, raw, st["raw_diff"] + st["ring"])
        return out

    fig_time_stack(
        series(lab_on0, ip_on0),
        "figA-stack-filter-on.svg",
        "Edit on frame 0, camera clip with the smoothing filter",
        "The rewrite spreads forward through the clip and never settles. "
        "9,026 raw blocks, file 3.5 MB against a 318 KB original.",
        "Counts exclude the 25 edited blocks. Every later frame copies patches from a "
        "picture that is no longer the camera’s, so more blocks open.",
    )
    fig_time_stack(
        series(lab_off0, ip_off0),
        "figB-stack-filter-off.svg",
        "Edit on frame 0, camera clip without the smoothing filter",
        "The rewrite stops after one frame. 468 raw blocks, file 478 KB. "
        "Frames 2–7 are byte-identical copies of the camera file.",
        "Counts exclude the 25 edited blocks. Raw blocks restore the camera’s exact "
        "pixels, so the next reference picture matches and the chain ends.",
    )

    print("raw-block map…")
    fig_raw_map(lab_off, load_ipcm(need["dfoff_ipcm7"]), frame=7)

    print("schematics…")
    fig_samples()
    fig_cascade_mechanism()
    fig_deblock_ring()
    fig_motion_vector()
    fig_cabac_bug()
    fig_deblock_pixels()
    fig_protocol()

    print("wrote", OUT)
    summary = {
        "frame7_filter_on": st_on,
        "frame7_filter_off": st_off,
    }
    (OUT / "figure-data.json").write_text(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
