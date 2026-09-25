#!/usr/bin/env bash
# Download a short 720p H.264 sample and convert to raw YUV420p for neighbor experiments.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/docs/fixtures/sample_720p_30f.yuv"
TMP="$(mktemp -t eva-sample-XXXX.mp4)"
mkdir -p "$(dirname "$OUT")"

echo "Fetching 720p sample MP4..."
curl -fsSL -o "$TMP" "https://filesamples.com/samples/video/mp4/sample_1280x720.mp4"

echo "Converting to 30-frame YUV420p → $OUT"
ffmpeg -y -hide_banner -loglevel error -i "$TMP" -pix_fmt yuv420p -frames:v 30 "$OUT"

rm -f "$TMP"
ls -lh "$OUT"
echo "Done. Run: cargo run --release -p video --example experiment_matrix -- --out docs/results/neighbor-matrix-720p.csv"
