#!/usr/bin/env python3
"""Compare two Eva dump sets record by record.

Used to prove that dumps recovered from a bitstream by the instrumented decoder
match the dumps our encoder wrote while producing that bitstream. Reports, per
file, how many macroblock records differ and where the first difference sits, so
a wrong field shows up as a single failing file rather than a wall of bytes.

Usage: compare_dumps.py ENCODER_DIR DECODER_DIR [MBS_PER_FRAME]
"""
import struct
import sys
from pathlib import Path

RECORDS = {
    "type_enc": 6,
    "mv_enc": 96,
    "mvd_enc": 64,
    "intra_modes_enc": 16,
    "b8x8_enc": 8,
    "pred_y_enc": 256,
    "coeff_y_enc": 256,
    "luma_cof_enc": 4 + 8 + 2 * 18 * 4 + 4 * 4 * 2 * 65 * 4,
    "chroma_enc": 2 + 2 * (2 * 18 * 4) + 2 * 4 * (2 * 65 * 4),
}

TYPE_FIELDS = ["slice_kind", "mb_type", "transform8x8", "intra_mode0", "qp", "qpc"]


def type_field_breakdown(a, b, size):
    """Which of the six type_enc bytes disagree, and how often."""
    counts = [0] * size
    for off in range(0, min(len(a), len(b)), size):
        ra, rb = a[off:off + size], b[off:off + size]
        if ra != rb:
            for i in range(size):
                if ra[i] != rb[i]:
                    counts[i] += 1
    return counts


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    enc, dec = Path(sys.argv[1]), Path(sys.argv[2])
    mbs_per_frame = int(sys.argv[3]) if len(sys.argv) > 3 else 3600

    worst = 0
    for name, size in RECORDS.items():
        ea, da = enc / name, dec / name
        if not ea.exists():
            print(f"{name:18s} SKIP (no encoder dump)")
            continue
        if not da.exists():
            print(f"{name:18s} MISSING from decoder dumps")
            worst = max(worst, 2)
            continue
        a, b = ea.read_bytes(), da.read_bytes()
        n = min(len(a), len(b)) // size
        differ = []
        for r in range(n):
            off = r * size
            if a[off:off + size] != b[off:off + size]:
                differ.append(r)
        note = ""
        if len(a) != len(b):
            note = f"  [length {len(a)} vs {len(b)}]"
        if not differ:
            print(f"{name:18s} OK   {n:6d}/{n} records identical{note}")
            continue
        worst = max(worst, 1)
        first = differ[0]
        frame, mb = divmod(first, mbs_per_frame)
        pct = 100.0 * len(differ) / n
        print(f"{name:18s} DIFF {len(differ):6d}/{n} records ({pct:.1f}%), "
              f"first at frame {frame} mb {mb}{note}")
        if name == "type_enc":
            counts = type_field_breakdown(a, b, size)
            detail = ", ".join(f"{TYPE_FIELDS[i]}={counts[i]}"
                               for i in range(size) if counts[i])
            print(f"{'':18s}      fields: {detail}")
            off = first * size
            print(f"{'':18s}      enc {list(a[off:off+size])} vs "
                  f"dec {list(b[off:off+size])}")
        elif name in ("luma_cof_enc", "chroma_enc"):
            off = first * size
            ints_a = struct.unpack_from("<3i", a, off)
            ints_b = struct.unpack_from("<3i", b, off)
            print(f"{'':18s}      first record head enc {ints_a} vs dec {ints_b}")
    return worst


if __name__ == "__main__":
    sys.exit(main())
