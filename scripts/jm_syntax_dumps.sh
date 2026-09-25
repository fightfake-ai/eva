#!/usr/bin/env bash
# Build JM lencod with Eva syntax dump hook and encode a YUV → pred_y_enc/coeff_y_enc/type_enc.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JM_SRC="${JM_SRC:-/tmp/JM}"
EVA_HOOK="$ROOT/third_party/jm-eva"
OUT_DIR=""
YUV=""
WIDTH=1280
HEIGHT=720
FRAMES=30
BFRAMES=0
GOP=""
ALL_INTRA=0
CONSTRAINED=0
INDEPENDENT_FRAMES=0
I4_ONLY=0
QP_I=28
QP_P=30
QP_B=28
STRICT_CQP=0
REFS=""
LOSSLESS=0
PROFILE=100
DF_OFF=0
RATE_KBPS=""

usage() {
  cat <<EOF
Usage: $(basename "$0") --yuv FILE --out DIR [--width W] [--height H] [--frames N] [--bframes N]
       [--gop N] [--all-intra] [--constrained] [--independent-frames] [--i4-only] [--strict-cqp]
       [--qp N] [--refs N] [--lossless] [--profile N]

Encodes raw YUV420p with JM and writes Eva-style syntax dumps:
  pred_y_enc  coeff_y_enc  type_enc  mv_enc  intra_modes_enc  b8x8_enc

Default: Main profile, IPPP (BFrames=0), CQP I/P=28/30, long GOP (I at frame 0 only).
--i4-only: Transform8x8Mode=0 DisableIntra16x16=1 (I4MB only on I slices).
--refs N: NumberReferenceFrames (default: encoder.cfg, usually 5).
--lossless: LosslessCoding=1 and ProfileIDC=244 (High 4:4:4).
--df-off: disable the deblocking filter on I/P slices (capture-side policy).
--rate-kbps N: enable rate control at N kbit/s (QP then varies per frame).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yuv) YUV="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --width) WIDTH="$2"; shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --bframes) BFRAMES="$2"; shift 2 ;;
    --gop) GOP="$2"; shift 2 ;;
    --all-intra) ALL_INTRA=1; shift ;;
    --constrained) CONSTRAINED=1; shift ;;
    --independent-frames) INDEPENDENT_FRAMES=1; shift ;;
    --i4-only) I4_ONLY=1; shift ;;
    --strict-cqp) STRICT_CQP=1; shift ;;
    --qp)
      QP_I="$2"; QP_P="$2"; QP_B="$2"
      shift 2
      ;;
    --refs) REFS="$2"; shift 2 ;;
    --lossless) LOSSLESS=1; PROFILE=244; shift ;;
    --df-off) DF_OFF=1; shift ;;
    --rate-kbps) RATE_KBPS="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1"; usage; exit 1 ;;
  esac
done

if [[ -z "$YUV" || -z "$OUT_DIR" ]]; then
  usage
  exit 1
fi
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
YUV="$(cd "$(dirname "$YUV")" && pwd)/$(basename "$YUV")"
if [[ ! -f "$YUV" ]]; then
  echo "missing yuv: $YUV" >&2
  exit 1
fi

if [[ ! -d "$JM_SRC/lencod" ]]; then
  echo "Cloning JM into $JM_SRC ..."
  git clone --depth 1 https://github.com/linzhenan/JM.git "$JM_SRC"
fi

cp "$EVA_HOOK/eva_dump.c" "$EVA_HOOK/eva_dump.h" "$EVA_HOOK/eva_glue.c" "$EVA_HOOK/eva_glue.h" "$JM_SRC/lencod/src/"
cp "$EVA_HOOK/eva_dump.h" "$EVA_HOOK/eva_glue.h" "$JM_SRC/lencod/inc/"
rm -f "$JM_SRC/lencod/obj/eva_dump.o" "$JM_SRC/lencod/obj/eva_glue.o"

SLICE="$JM_SRC/lencod/src/slice.c"
if ! grep -q eva_dump_macroblock "$SLICE"; then
  sed -i.bak 's/^#include "slice.h"/#include "slice.h"\n#include "eva_dump.h"/' "$SLICE"
  sed -i.bak 's/write_macroblock (currMB, 1);/write_macroblock (currMB, 1);\n      eva_dump_macroblock(currMB);/' "$SLICE"
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
if ! grep -q eva_dump_open "$MAIN"; then
  sed -i.bak '/^#include "global.h"/a\
#include "eva_dump.h"
' "$MAIN"
  python3 - "$MAIN" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
if "eva_dump_open" in text:
    sys.exit(0)
needle = "  init_encoder(p_Enc->p_Vid, p_Enc->p_Inp);"
insert = needle + "\n\n  {\n    const char *eva_dir = getenv(\"EVA_DUMP_DIR\");\n    if (eva_dir && eva_dir[0]) eva_dump_open(eva_dir);\n  }"
text = text.replace(needle, insert, 1)
text = text.replace("  free_encoder(p_Enc);", "  eva_dump_close();\n  free_encoder(p_Enc);", 1)
open(path, "w").write(text)
PY
fi

make -C "$JM_SRC/lencod" -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)" default 2>/dev/null || true
if [[ ! -x "$JM_SRC/bin/lencod.exe" ]]; then
  make -C "$JM_SRC/lencod" default
fi

mkdir -p "$OUT_DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

INTRA_PERIOD=0
IDR_PERIOD=0
if [[ "$ALL_INTRA" == "1" ]]; then
  INTRA_PERIOD=1
  IDR_PERIOD=1
elif [[ -n "$GOP" ]]; then
  INTRA_PERIOD="$GOP"
  IDR_PERIOD="$GOP"
fi

JM_EXTRA=(
  -p NumberBFrames="$BFRAMES"
  -p ProfileIDC="$PROFILE"
  -p QPISlice="$QP_I"
  -p QPPSlice="$QP_P"
  -p QPBSlice="$QP_B"
  -p UseConstrainedIntraPred="$CONSTRAINED"
  -p IntraPeriod="$INTRA_PERIOD"
  -p IDRPeriod="$IDR_PERIOD"
  -p AdaptiveIntraPeriod=0
  # Glue recon is predictor-only; distortion-based RPLM would diverge from dump.
  -p ReferenceReorder=0
  -p UseDistortionReorder=0
)

if [[ -n "$RATE_KBPS" ]]; then
  # JM refuses rate control unless RCUpdateMode is 2 or 3 with this base config.
  JM_EXTRA+=(
    -p RateControlEnable=1
    -p Bitrate=$(( RATE_KBPS * 1000 ))
    -p RCUpdateMode=3
  )
else
  JM_EXTRA+=(-p RateControlEnable=0)
fi

if [[ "$BFRAMES" != "0" ]]; then
  # The base config carries an explicit B hierarchy; plain IbbP needs it switched off.
  JM_EXTRA+=(-p HierarchicalCoding=0)
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

if [[ "$STRICT_CQP" == "1" ]]; then
  JM_EXTRA+=(
    -p HierarchyLevelQPEnable=0
    -p RDPictureFrameQPPSlice=0
    -p RDPictureFrameQPBSlice=0
  )
fi

if [[ "$I4_ONLY" == "1" ]]; then
  JM_EXTRA+=(
    -p Transform8x8Mode=0
    -p DisableIntra16x16=1
  )
fi

if [[ -n "$REFS" ]]; then
  JM_EXTRA+=(-p NumberReferenceFrames="$REFS")
fi

if [[ "$LOSSLESS" == "1" ]]; then
  JM_EXTRA+=(-p LosslessCoding=1)
fi

run_lencod_once() {
  local yuv_file="$1"
  local nframes="$2"
  local dump_dir="$3"
  local tag="$4"
  export EVA_DUMP_DIR="$dump_dir"
  mkdir -p "$dump_dir"
  cd "$WORK"
  "$JM_SRC/bin/lencod.exe" -d "$JM_SRC/bin/encoder.cfg" \
    -p InputFile="$yuv_file" \
    -p SourceWidth="$WIDTH" -p SourceHeight="$HEIGHT" \
    -p OutputWidth="$WIDTH" -p OutputHeight="$HEIGHT" \
    -p FramesToBeEncoded="$nframes" \
    -p StartFrame=0 \
    "${JM_EXTRA[@]}" \
    -p OutputFile="$WORK/out_${tag}.264" \
    -p ReconFile="$WORK/rec_${tag}.yuv"
}

if [[ "$INDEPENDENT_FRAMES" == "1" ]]; then
  frame_bytes=$(( WIDTH * HEIGHT * 3 / 2 ))
  : > "$OUT_DIR/pred_y_enc"
  : > "$OUT_DIR/coeff_y_enc"
  : > "$OUT_DIR/type_enc"
  : > "$OUT_DIR/mv_enc"
  : > "$OUT_DIR/intra_modes_enc"
  : > "$OUT_DIR/b8x8_enc"
  : > "$OUT_DIR/chroma_enc"
  : > "$OUT_DIR/mvd_enc"
  : > "$OUT_DIR/luma_cof_enc"
  for ((f=0; f<FRAMES; f++)); do
    dd if="$YUV" of="$WORK/frame.yuv" bs="$frame_bytes" skip="$f" count=1 status=none
    single="$WORK/dump_f${f}"
    run_lencod_once "$WORK/frame.yuv" 1 "$single" "f${f}"
    cat "$single/pred_y_enc" >> "$OUT_DIR/pred_y_enc"
    cat "$single/coeff_y_enc" >> "$OUT_DIR/coeff_y_enc"
    cat "$single/type_enc" >> "$OUT_DIR/type_enc"
    cat "$single/mv_enc" >> "$OUT_DIR/mv_enc"
    cat "$single/intra_modes_enc" >> "$OUT_DIR/intra_modes_enc"
    cat "$single/b8x8_enc" >> "$OUT_DIR/b8x8_enc"
    cat "$single/chroma_enc" >> "$OUT_DIR/chroma_enc"
    cat "$single/mvd_enc" >> "$OUT_DIR/mvd_enc"
    cat "$single/luma_cof_enc" >> "$OUT_DIR/luma_cof_enc"
  done
else
  run_lencod_once "$YUV" "$FRAMES" "$OUT_DIR" "full"
  if [[ -f "$WORK/out_full.264" ]]; then
    cp "$WORK/out_full.264" "$OUT_DIR/source.264"
  fi
fi

echo "Wrote syntax dumps → $OUT_DIR"
wc -c "$OUT_DIR"/pred_y_enc "$OUT_DIR"/coeff_y_enc "$OUT_DIR"/type_enc "$OUT_DIR"/mv_enc \
  "$OUT_DIR"/intra_modes_enc "$OUT_DIR"/b8x8_enc "$OUT_DIR"/chroma_enc "$OUT_DIR"/mvd_enc \
  "$OUT_DIR"/luma_cof_enc 2>/dev/null || true
if [[ -f "$OUT_DIR/source.264" ]]; then
  wc -c "$OUT_DIR/source.264"
fi
