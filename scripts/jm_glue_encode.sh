#!/usr/bin/env bash
# Encode YUV with JM using Eva CABAC glue (inject merged syntax per MB, write_macroblock only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JM_SRC="${JM_SRC:-/tmp/JM}"
EVA_HOOK="$ROOT/third_party/jm-eva"
GLUE_DIR=""
YUV=""
OUT_264=""
WIDTH=1280
HEIGHT=720
FRAMES=30
BFRAMES=0
GOP=""
I4_ONLY=0
ALL_INTRA=0
QP_I=28
QP_P=30
QP_B=28
RECON=""
DF_OFF=0
REFS=""
LOSSLESS=0
PROFILE=100
RATE_KBPS=""
CHROMA_QP_OFFSET=""

usage() {
  cat <<EOF
Usage: $(basename "$0") --glue-dir DIR --yuv FILE --out FILE.264 [options]

Encodes with EVA_GLUE_DIR set: each MB is injected from glue dumps then CABAC-written.
Requires pred_y_enc, coeff_y_enc, type_enc, mv_enc in GLUE_DIR; intra_modes_enc and b8x8_enc optional.
--i4-only: Transform8x8Mode=0 DisableIntra16x16=1 (match I4-only dumps).
--all-intra: IntraPeriod=IDRPeriod=1 (match all-intra dumps).
--df-off: disable the deblocking filter on I/P slices.
--recon FILE: keep JM's reconstructed YUV (otherwise it is deleted with the temp dir).
--refs N: NumberReferenceFrames (must match the dumps).
--rate-kbps N: accepted for symmetry with the capture; the replay takes its
  per-frame QP from the dumps, so it never runs a rate controller.
--chroma-qp-offset N: must match the source stream's chroma_qp_index_offset; the
  bitstream carries it in the PPS, not per macroblock (x264 defaults to -2).
--lossless: LosslessCoding=1 and ProfileIDC=244 (High 4:4:4), for QP-0 transform bypass.
--profile N: ProfileIDC override (default 100, or 244 with --lossless).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --glue-dir) GLUE_DIR="$2"; shift 2 ;;
    --yuv) YUV="$2"; shift 2 ;;
    --out) OUT_264="$2"; shift 2 ;;
    --width) WIDTH="$2"; shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --bframes) BFRAMES="$2"; shift 2 ;;
    --gop) GOP="$2"; shift 2 ;;
    --i4-only) I4_ONLY=1; shift ;;
    --all-intra) ALL_INTRA=1; shift ;;
    --df-off) DF_OFF=1; shift ;;
    --recon) RECON="$2"; shift 2 ;;
    --qp) QP_I="$2"; QP_P="$2"; QP_B="$2"; shift 2 ;;
    --refs) REFS="$2"; shift 2 ;;
    --rate-kbps) RATE_KBPS="$2"; shift 2 ;;
    --chroma-qp-offset) CHROMA_QP_OFFSET="$2"; shift 2 ;;
    --lossless) LOSSLESS=1; PROFILE=244; shift ;;
    --profile) PROFILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1"; usage; exit 1 ;;
  esac
done

if [[ -z "$GLUE_DIR" || -z "$YUV" || -z "$OUT_264" ]]; then
  usage
  exit 1
fi
for f in pred_y_enc coeff_y_enc type_enc mv_enc; do
  if [[ ! -f "$GLUE_DIR/$f" ]]; then
    echo "missing $GLUE_DIR/$f" >&2
    exit 1
  fi
done

mkdir -p "$(dirname "$OUT_264")"
GLUE_DIR="$(cd "$GLUE_DIR" && pwd)"
YUV="$(cd "$(dirname "$YUV")" && pwd)/$(basename "$YUV")"
OUT_264="$(cd "$(dirname "$OUT_264")" && pwd)/$(basename "$OUT_264")"
if [[ -n "$RECON" ]]; then
  mkdir -p "$(dirname "$RECON")"
  RECON="$(cd "$(dirname "$RECON")" && pwd)/$(basename "$RECON")"
fi

if [[ ! -d "$JM_SRC/lencod" ]]; then
  echo "Cloning JM into $JM_SRC ..."
  git clone --depth 1 https://github.com/linzhenan/JM.git "$JM_SRC"
fi

cp "$EVA_HOOK/eva_dump.c" "$EVA_HOOK/eva_dump.h" "$EVA_HOOK/eva_glue.c" "$EVA_HOOK/eva_glue.h" "$JM_SRC/lencod/src/"
cp "$EVA_HOOK/eva_dump.h" "$EVA_HOOK/eva_glue.h" "$JM_SRC/lencod/inc/"

SLICE="$JM_SRC/lencod/src/slice.c"
if ! grep -q eva_glue_macroblock "$SLICE"; then
  if ! grep -q '#include "eva_glue.h"' "$SLICE"; then
    sed -i.bak 's/^#include "slice.h"/#include "slice.h"\n#include "eva_glue.h"/' "$SLICE"
  fi
  python3 - "$SLICE" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
old_dump = """      currSlice->encode_one_macroblock (currMB);
      end_encode_one_macroblock(currMB);

      write_macroblock (currMB, 1);
      eva_dump_macroblock(currMB);"""
old_clean = """      currSlice->encode_one_macroblock (currMB);
      end_encode_one_macroblock(currMB);

      write_macroblock (currMB, 1);"""
new_dump = """      if (eva_glue_enabled()) {
        if (eva_glue_should_inject(currMB))
          eva_glue_macroblock(currMB);
        else
          currSlice->encode_one_macroblock (currMB);
        end_encode_one_macroblock(currMB);
        write_macroblock (currMB, 1);
      } else {
        currSlice->encode_one_macroblock (currMB);
        end_encode_one_macroblock(currMB);
        write_macroblock (currMB, 1);
        eva_dump_macroblock(currMB);
      }"""
new_clean = """      if (eva_glue_enabled()) {
        if (eva_glue_should_inject(currMB))
          eva_glue_macroblock(currMB);
        else
          currSlice->encode_one_macroblock (currMB);
        end_encode_one_macroblock(currMB);
        write_macroblock (currMB, 1);
      } else {
        currSlice->encode_one_macroblock (currMB);
        end_encode_one_macroblock(currMB);
        write_macroblock (currMB, 1);
      }"""
if old_dump in text:
    text = text.replace(old_dump, new_dump, 1)
elif old_clean in text:
    text = text.replace(old_clean, new_clean, 1)
else:
    sys.exit("slice.c encode loop pattern not found")
open(path, "w").write(text)
PY
fi

# Rate-controlled captures move the frame QP; the replay has no rate controller,
# so take the slice header QP from the dumps.
if ! grep -q eva_glue_slice_qp "$SLICE"; then
  if ! grep -q '#include "eva_glue.h"' "$SLICE"; then
    sed -i.bak 's/^#include "slice.h"/#include "slice.h"\n#include "eva_glue.h"/' "$SLICE"
  fi
  python3 - "$SLICE" <<'PY2'
import sys
path = sys.argv[1]
text = open(path).read()
old = """  currSlice->qp                = p_Vid->p_curr_frm_struct->qp;
  currSlice->start_mb_nr       = p_Vid->current_mb_nr;"""
new = """  currSlice->qp                = p_Vid->p_curr_frm_struct->qp;
  currSlice->start_mb_nr       = p_Vid->current_mb_nr;
  if (eva_glue_enabled())
  {
    int eva_qp = eva_glue_slice_qp(p_Vid->frame_no, currSlice->start_mb_nr);
    if (eva_qp >= 0)
      currSlice->qp = (short) eva_qp;
  }"""
if old not in text:
    sys.exit("slice.c set_slice QP pattern not found")
open(path, "w").write(text.replace(old, new, 1))
PY2
fi

MAIN="$JM_SRC/lencod/src/lencod.c"
if ! grep -q eva_glue_open "$MAIN"; then
  sed -i.bak '/^#include "global.h"/a\
#include "eva_glue.h"
' "$MAIN"
  python3 - "$MAIN" "$WIDTH" "$HEIGHT" <<'PY'
import sys
path, w, h = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path).read()
if "eva_glue_open" in text:
    sys.exit(0)
needle = "  init_encoder(p_Enc->p_Vid, p_Enc->p_Inp);"
insert = needle + f"""
  {{
    const char *glue_dir = getenv(\"EVA_GLUE_DIR\");
    if (glue_dir && glue_dir[0])
      eva_glue_open(glue_dir, {w}, {h});
  }}"""
text = text.replace(needle, insert, 1)
text = text.replace("  free_encoder(p_Enc);", "  eva_glue_close();\n  free_encoder(p_Enc);", 1)
open(path, "w").write(text)
PY
fi

# When glue provides dumped MVDs, write those instead of recomputing from GetMVPredictor
# (partition predictors can differ from the original encode even when absolute MVs match).
MBC="$JM_SRC/lencod/src/macroblock.c"
if ! grep -q eva_glue_use_stored_mvd "$MBC"; then
  if ! grep -q '#include "eva_glue.h"' "$MBC"; then
    sed -i.bak 's/^#include "macroblock.h"/#include "macroblock.h"\n#include "eva_glue.h"/' "$MBC"
  fi
  python3 - "$MBC" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
old = """      // Lets recompute MV predictor. This should avoid any problems with alterations of the motion vectors after ME
      get_neighbors(currMB, block, i<<2, j<<2, step_h<<2);
      currMB->GetMVPredictor (currMB, block, &predMV, (short) refindex, p_Vid->enc_picture->mv_info, list_idx, (i<<2), (j<<2), step_h<<2, step_v<<2);
      //test_clip_mvs(p_Vid, cur_mv, currMB->write_mb);
      mvd[0] = cur_mv->mv_x - predMV.mv_x;
      mvd[1] = cur_mv->mv_y - predMV.mv_y;"""
new = """      // Lets recompute MV predictor. This should avoid any problems with alterations of the motion vectors after ME
      get_neighbors(currMB, block, i<<2, j<<2, step_h<<2);
      currMB->GetMVPredictor (currMB, block, &predMV, (short) refindex, p_Vid->enc_picture->mv_info, list_idx, (i<<2), (j<<2), step_h<<2, step_v<<2);
      //test_clip_mvs(p_Vid, cur_mv, currMB->write_mb);
      if (eva_glue_use_stored_mvd()) {
        mvd[0] = currMB_mvd[j][i][0];
        mvd[1] = currMB_mvd[j][i][1];
      } else {
        mvd[0] = cur_mv->mv_x - predMV.mv_x;
        mvd[1] = cur_mv->mv_y - predMV.mv_y;
      }"""
if old not in text:
    sys.exit("writeMotionVector8x8 MVD pattern not found in macroblock.c")
text = text.replace(old, new, 1)
open(path, "w").write(text)
PY
fi

rm -f "$JM_SRC/lencod/obj/eva_glue.o" "$JM_SRC/lencod/obj/eva_dump.o" "$JM_SRC/lencod/obj/macroblock.o" "$JM_SRC/lencod/obj/slice.o"

make -C "$JM_SRC/lencod" -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)" default 2>/dev/null || true
if [[ ! -x "$JM_SRC/bin/lencod.exe" ]]; then
  make -C "$JM_SRC/lencod" default
fi

INTRA_PERIOD=0
IDR_PERIOD=0
if [[ "$ALL_INTRA" == "1" ]]; then
  INTRA_PERIOD=1
  IDR_PERIOD=1
elif [[ -n "$GOP" ]]; then
  INTRA_PERIOD="$GOP"
  IDR_PERIOD="$GOP"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export EVA_GLUE_DIR="$GLUE_DIR"
JM_EXTRA=()
if [[ "$I4_ONLY" == "1" ]]; then
  JM_EXTRA+=(
    -p Transform8x8Mode=0
    -p DisableIntra16x16=1
  )
fi
if [[ "$DF_OFF" == "1" ]]; then
  JM_EXTRA+=(
    -p DFParametersFlag=1
    -p DFDisableRefISlice=1
    -p DFDisableNRefISlice=1
    -p DFDisableRefPSlice=1
    -p DFDisableNRefPSlice=1
  )
fi
if [[ -n "$REFS" ]]; then
  JM_EXTRA+=(-p NumberReferenceFrames="$REFS")
fi
if [[ "$BFRAMES" != "0" ]]; then
  # Must match the dump encode: plain IbbP, no explicit B hierarchy.
  JM_EXTRA+=(-p HierarchicalCoding=0)
fi
if [[ "$LOSSLESS" == "1" ]]; then
  JM_EXTRA+=(-p LosslessCoding=1)
fi
JM_EXTRA+=(-p RateControlEnable=0)
if [[ -n "$CHROMA_QP_OFFSET" ]]; then
  # JM picks the Cb/Cr pair or the single offset depending on the colour format,
  # so set all three and let it use whichever it writes into the PPS.
  JM_EXTRA+=(
    -p ChromaQPOffset="$CHROMA_QP_OFFSET"
    -p CbQPOffset="$CHROMA_QP_OFFSET"
    -p CrQPOffset="$CHROMA_QP_OFFSET"
  )
fi
cd "$WORK"
# bash 3.2 + set -u: empty "${arr[@]}" errors — expand only when non-empty
set -- \
  -p InputFile="$YUV" \
  -p SourceWidth="$WIDTH" -p SourceHeight="$HEIGHT" \
  -p OutputWidth="$WIDTH" -p OutputHeight="$HEIGHT" \
  -p FramesToBeEncoded="$FRAMES" \
  -p StartFrame=0 \
  -p NumberBFrames="$BFRAMES" \
  -p ProfileIDC="$PROFILE" \
  -p QPISlice="$QP_I" \
  -p QPPSlice="$QP_P" \
  -p QPBSlice="$QP_B" \
  -p IntraPeriod="$INTRA_PERIOD" \
  -p IDRPeriod="$IDR_PERIOD" \
  -p AdaptiveIntraPeriod=0 \
  -p ReferenceReorder=0 \
  -p UseDistortionReorder=0 \
  -p OutputFile="$OUT_264" \
  -p ReconFile="$WORK/rec.yuv"
if ((${#JM_EXTRA[@]})); then
  set -- "$@" "${JM_EXTRA[@]}"
fi
"$JM_SRC/bin/lencod.exe" -d "$JM_SRC/bin/encoder.cfg" "$@"

if [[ -n "$RECON" ]]; then
  mkdir -p "$(dirname "$RECON")"
  cp "$WORK/rec.yuv" "$RECON"
  echo "Wrote $RECON ($(wc -c < "$RECON") bytes)"
fi

echo "Wrote $OUT_264 ($(wc -c < "$OUT_264") bytes)"
