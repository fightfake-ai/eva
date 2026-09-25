#!/usr/bin/env python3
"""Mixed-bitstream publish path: redact a YUV, splice JM syntax dumps per MB, compare decodes.

Every Eva sidecar (`pred_y_enc`, `coeff_y_enc`, `type_enc`, `mv_enc`, `intra_modes_enc`,
`b8x8_enc`, `chroma_enc`, `mvd_enc`, `luma_cof_enc`) is a fixed-size record per macroblock,
so a merge is a per-slot byte choice: island slots come from the edited encode, everything
else from the original capture encode. Record widths are derived from file size / MB count
rather than hardcoded, so the tool tracks changes in `third_party/jm-eva/eva_dump.c`.

  redact  --yuv IN --out OUT --frames 14 16 --box 320 180 640 360
  merge   --orig DIR --edited DIR --slots FILE --out DIR

Scoring the decoded result is `video/examples/mixed_publish_report.rs`.
"""

import argparse
import os
import sys

SIDECARS = [
    "pred_y_enc",
    "coeff_y_enc",
    "type_enc",
    "mv_enc",
    "intra_modes_enc",
    "b8x8_enc",
    "chroma_enc",
    "mvd_enc",
    "luma_cof_enc",
]


def frame_bytes(w, h):
    return w * h * 3 // 2


def cmd_redact(a):
    w, h = a.width, a.height
    fb = frame_bytes(w, h)
    data = bytearray(open(a.yuv, "rb").read())
    nframes = len(data) // fb
    x, y, bw, bh = a.box
    f0, f1 = a.frames
    for f in range(max(0, f0), min(f1, nframes)):
        base = f * fb
        for row in range(y, min(y + bh, h)):
            s = base + row * w + x
            data[s : s + min(bw, w - x)] = b"\x00" * min(bw, w - x)
        cw, ch = w // 2, h // 2
        cx, cy, cbw, cbh = x // 2, y // 2, bw // 2, bh // 2
        for plane in (0, 1):
            pbase = base + w * h + plane * cw * ch
            for row in range(cy, min(cy + cbh, ch)):
                s = pbase + row * cw + cx
                data[s : s + min(cbw, cw - cx)] = b"\x80" * min(cbw, cw - cx)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    open(a.out, "wb").write(bytes(data))
    print(f"redact: {a.out} ({len(data)} B, {nframes} frames, box={a.box}, frames={f0}..{f1})")


def mb_count(d):
    return os.path.getsize(os.path.join(d, "type_enc")) // 6


def cmd_merge(a):
    n_orig, n_edit = mb_count(a.orig), mb_count(a.edited)
    if n_orig != n_edit:
        sys.exit(f"MB count mismatch: orig {n_orig} vs edited {n_edit}")
    slots = sorted({int(l) for l in open(a.slots) if l.strip() and not l.startswith("#")})
    os.makedirs(a.out, exist_ok=True)

    for name in SIDECARS:
        po, pe = os.path.join(a.orig, name), os.path.join(a.edited, name)
        if not (os.path.exists(po) and os.path.exists(pe)):
            print(f"  skip {name} (missing in one side)")
            continue
        so, se = os.path.getsize(po), os.path.getsize(pe)
        if so != se or so % n_orig:
            sys.exit(f"{name}: size {so}/{se} not a clean multiple of {n_orig} MBs")
        rec = so // n_orig
        buf = bytearray(open(po, "rb").read())
        edit = open(pe, "rb").read()
        changed = 0
        for g in slots:
            lo, hi = g * rec, (g + 1) * rec
            if buf[lo:hi] != edit[lo:hi]:
                changed += 1
            buf[lo:hi] = edit[lo:hi]
        open(os.path.join(a.out, name), "wb").write(bytes(buf))
        print(f"  {name}: {rec} B/MB, {changed}/{len(slots)} island slots differed")

    print(f"merge: {len(slots)} island slots of {n_orig} → {a.out}")


def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("redact")
    r.add_argument("--yuv", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--width", type=int, default=1280)
    r.add_argument("--height", type=int, default=720)
    r.add_argument("--frames", type=int, nargs=2, default=[14, 16])
    r.add_argument("--box", type=int, nargs=4, default=[320, 180, 640, 360])
    r.set_defaults(func=cmd_redact)

    m = sub.add_parser("merge")
    m.add_argument("--orig", required=True)
    m.add_argument("--edited", required=True)
    m.add_argument("--slots", required=True)
    m.add_argument("--out", required=True)
    m.set_defaults(func=cmd_merge)

    a = p.parse_args()
    a.func(a)


if __name__ == "__main__":
    main()
