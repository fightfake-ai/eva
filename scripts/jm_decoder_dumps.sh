#!/usr/bin/env bash
# Produce Eva per-macroblock syntax dumps from an arbitrary H.264 file by
# instrumenting the JM *decoder*. Output is byte-compatible with the dumps
# scripts/jm_syntax_dumps.sh takes off our patched encoder, so the publishing
# glue cannot tell the two apart.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JM_SRC="${JM_SRC:-/tmp/JM-eva-src}"
EVA_HOOK="$ROOT/third_party/jm-eva"
IN_264=""
OUT_DIR=""
RECON=""

usage() {
  cat <<EOF
Usage: $(basename "$0") --in FILE.264 --out DIR [--recon FILE.yuv]

Decodes FILE.264 and writes per-macroblock syntax dumps into DIR:
  pred_y_enc coeff_y_enc type_enc mv_enc intra_modes_enc b8x8_enc
  chroma_enc mvd_enc luma_cof_enc
--recon FILE: also keep the decoded YUV (this is the camera's picture).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --in) IN_264="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --recon) RECON="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1"; usage; exit 1 ;;
  esac
done

if [[ -z "$IN_264" || -z "$OUT_DIR" ]]; then
  usage
  exit 1
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
IN_264="$(cd "$(dirname "$IN_264")" && pwd)/$(basename "$IN_264")"

if [[ ! -d "$JM_SRC/ldecod" ]]; then
  echo "Cloning JM into $JM_SRC ..."
  git clone --depth 1 https://github.com/linzhenan/JM.git "$JM_SRC"
fi

cp "$EVA_HOOK/eva_decdump.c" "$EVA_HOOK/eva_decdump.h" "$JM_SRC/ldecod/src/"
cp "$EVA_HOOK/eva_decdump.h" "$JM_SRC/ldecod/inc/"

python3 - "$JM_SRC" <<'PY'
"""Insert the dump hooks into the decoder.

Every hook is an append-only capture; none of them changes a decoded value. The
coefficient hooks have to sit at parse time because the decoder dequantises its
coefficient array in place, so reading it back afterwards no longer yields the
quantised levels the bitstream carried.
"""
import sys, os, re

jm = sys.argv[1]
src = os.path.join(jm, "ldecod", "src")


def patch(name, edits, guard):
    path = os.path.join(src, name)
    text = open(path, encoding="latin-1").read()
    if guard in text:
        return "already patched"
    if '#include "eva_decdump.h"' not in text:
        m = re.search(r'#include "[^"]+\.h"\n', text)
        if not m:
            sys.exit(f"{name}: no include block to extend")
        text = text[:m.end()] + '#include "eva_decdump.h"\n' + text[m.end():]
    for old, new, count in edits:
        found = text.count(old)
        if found < count:
            sys.exit(f"{name}: expected >={count} of {old!r}, found {found}")
        text = text.replace(old, new, count)
    open(path, "w", encoding="latin-1").write(text)
    return "patched"


# --- the macroblock loop: reset capture, then emit the record ----------------
print("image.c:", patch(
    "image.c",
    [(
        """    start_macroblock(currSlice, &currMB);
    // Get the syntax elements from the NAL
    currSlice->read_one_macroblock(currMB);
    decode_one_macroblock(currMB, currSlice->dec_picture);""",
        """    start_macroblock(currSlice, &currMB);
    // Get the syntax elements from the NAL
    eva_decdump_begin_mb(currMB);
    currSlice->read_one_macroblock(currMB);
    decode_one_macroblock(currMB, currSlice->dec_picture);
    eva_decdump_macroblock(currMB);""",
        1,
    )],
    "eva_decdump_begin_mb",
))

# --- intra prediction modes: the coded value, not the absolute one -----------
print("mb_read.c:", patch(
    "mb_read.c",
    [
        # 4x4: nested b8 / j / i loops, so the encoder's index is 4*b8 + 2*j + i.
        # Two variants in the file (MBAFF and frame), identical loop variables.
        ("""        currSlice->ipredmode[bj][bi] = (byte) ((currSE.value1 == -1) ? mostProbableIntraPredMode : currSE.value1 + (currSE.value1 >= mostProbableIntraPredMode));""",
         """        eva_decdump_intra_mode(4 * b8 + (j << 1) + i, currSE.value1);
        currSlice->ipredmode[bj][bi] = (byte) ((currSE.value1 == -1) ? mostProbableIntraPredMode : currSE.value1 + (currSE.value1 >= mostProbableIntraPredMode));""",
         2),
        # 8x8: 4 blocks, each writing a 2x2 patch of 4x4 modes
        ("""    currSlice->ipredmode[bj    ][bi    ] = (byte) dec;""",
         """    eva_decdump_intra_mode(4 * b8, currSE.value1);
    currSlice->ipredmode[bj    ][bi    ] = (byte) dec;""",
         2),
    ],
    "eva_decdump_intra_mode",
))

# --- coefficients: capture (level, run) exactly as parsed --------------------
print("read_comp_cabac.c:", patch(
    "read_comp_cabac.c",
    [
        # 4x4 luma/444 block, first coefficient of the block
        ("""          cof[j + j0][i + i0]= level;""",
         """          eva_decdump_ac(2 * (j >> 3) + (i >> 3),
                         2 * ((j >> 2) & 1) + ((i >> 2) & 1),
                         level, currSE->value2);
          cof[j + j0][i + i0]= level;""",
         1),
        # 4x4 luma/444 block, remaining coefficients
        ("""            cof[j + j0][i + i0] = level;""",
         """            eva_decdump_ac(2 * (j >> 3) + (i >> 3),
                           2 * ((j >> 2) & 1) + ((i >> 2) & 1),
                           level, currSE->value2);
            cof[j + j0][i + i0] = level;""",
         1),
        # Intra16x16 luma DC
        ("""          cof[j0][i0] = level;// add new intra DC coeff""",
         """          eva_decdump_dc(0, level, currSE.value2);
          cof[j0][i0] = level;// add new intra DC coeff""",
         1),
        # chroma DC
        ("""          currSlice->cofu[coef_ctr] = level;""",
         """          eva_decdump_dc(uv + 1, level, currSE.value2);
          currSlice->cofu[coef_ctr] = level;""",
         1),
        # chroma AC, dequantising path
        ("""              cof[(j<<2) + j0][(i<<2) + i0] = rshift_rnd_sf((level * InvLevelScale4x4[j0][i0])<<qp_per_uv[uv], 4);""",
         """              eva_decdump_ac(4 + uv, b4, level, currSE.value2);
              cof[(j<<2) + j0][(i<<2) + i0] = rshift_rnd_sf((level * InvLevelScale4x4[j0][i0])<<qp_per_uv[uv], 4);""",
         1),
        # chroma AC, lossless path
        ("""              currSlice->cof[uv + 1][(j<<2) + j0][(i<<2) + i0] = level;""",
         """              eva_decdump_ac(4 + uv, b4, level, currSE.value2);
              currSlice->cof[uv + 1][(j<<2) + j0][(i<<2) + i0] = level;""",
         1),
        # 8x8 transform: all 64 pairs of a block live under b4 = 0.
        # First the block's DC, then its AC loop (note the differing whitespace).
        ("""      tcoeffs[j][boff_x + i] = rshift_rnd_sf((level * InvLevelScale8x8[j][i]) << qp_per, 6); // dequantization""",
         """      eva_decdump_ac(b8, 0, level, currSE->value2);
      tcoeffs[j][boff_x + i] = rshift_rnd_sf((level * InvLevelScale8x8[j][i]) << qp_per, 6); // dequantization""",
         1),
        ("""          tcoeffs[ j][boff_x + i] = rshift_rnd_sf((level * InvLevelScale8x8[j][i]) << qp_per, 6); // dequantization""",
         """          eva_decdump_ac(b8, 0, level, currSE->value2);
          tcoeffs[ j][boff_x + i] = rshift_rnd_sf((level * InvLevelScale8x8[j][i]) << qp_per, 6); // dequantization""",
         1),
    ],
    "eva_decdump_ac",
))
PY

MAKEFILE="$JM_SRC/ldecod/Makefile"
if ! grep -q eva_decdump "$MAKEFILE" 2>/dev/null; then
  # JM's Makefile globs src/*.c, so nothing to add; confirm the glob exists.
  grep -q 'wildcard' "$MAKEFILE" || echo "note: check that $MAKEFILE compiles src/*.c"
fi

make -C "$JM_SRC/ldecod" -j8 default >/dev/null

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

EVA_DECDUMP_DIR="$OUT_DIR" "$JM_SRC/bin/ldecod.exe" \
  -p InputFile="$IN_264" \
  -p OutputFile="$WORK/dec.yuv" \
  -p RefFile="" >"$OUT_DIR/decode.log" 2>&1 || {
    tail -20 "$OUT_DIR/decode.log"
    echo "ldecod failed" >&2
    exit 1
  }

if [[ -n "$RECON" ]]; then
  mkdir -p "$(dirname "$RECON")"
  cp "$WORK/dec.yuv" "$RECON"
fi

echo "wrote dumps to $OUT_DIR"
ls -l "$OUT_DIR" | tail -n +2
