# Video fixtures for neighbor experiments

Neighbor-impact tools default to **1280×720 × 30 frames**, not CIF.

## Generate the default 720p clip

```bash
./scripts/fetch_fixture_720p.sh
```

Produces `sample_720p_30f.yuv` (~40 MB, gitignored). Source: a short H.264 sample
re-encoded to raw YUV420p via ffmpeg.

## JM syntax dumps (Eva-style)

True syntax comparison uses JM encoder dumps, not ffmpeg decode:

```bash
./scripts/jm_syntax_dumps.sh \
  --yuv docs/fixtures/sample_720p_30f.yuv \
  --out docs/fixtures/jm-dumps-720p/orig \
  --width 1280 --height 720 --frames 30
```

Writes `pred_y_enc`, `coeff_y_enc`, `type_enc` (~56 MB; gitignored). First run
clones/builds [JM 19](https://github.com/linzhenan/JM) under `/tmp/JM` with an Eva
dump hook from `third_party/jm-eva/`.

Full walkthrough (orig + edited dumps + JSON):

```bash
cargo run --release -p video --example mb_walkthrough_export -- \
  --out docs/results/mb-walkthrough-720p-intra-syntax.json
```

## Run experiments

```bash
cargo run --release -p video --example experiment_matrix -- \
  --out docs/results/neighbor-matrix-720p.csv

cargo run --release -p video --example mb_grid_export -- \
  --syntax-orig-dir docs/fixtures/jm-dumps-720p/orig \
  --syntax-edit-dir docs/fixtures/jm-dumps-720p/edited \
  --out docs/results/mb-grid-720p-syntax.json
```

If the fixture file exists, tools load it automatically (no `--yuv` needed).

## Your own video

```bash
ffmpeg -i your.mp4 -vf scale=1280:720 -pix_fmt yuv420p -frames:v 30 your.yuv

./scripts/jm_syntax_dumps.sh --yuv your.yuv --out ./dumps/orig --width 1280 --height 720 --frames 30
```

Width and height must be multiples of 16.
