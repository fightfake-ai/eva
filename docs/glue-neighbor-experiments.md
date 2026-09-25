# Glue protocol and neighbor-impact experiments

This document ties together the **island geometry**, **Merkle glue protocol**, and **neighbor re-encode measurements** in the `video` crate. Use it as the entry point for running experiments and interpreting CSV output.

**Visual companions (open beside chat in Cursor):**

- [Glue protocol](/Users/miha/.cursor/projects/Users-miha-projects-fightfake-projects-eva-miha/canvases/glue-protocol.canvas.tsx) — capture → prove → verify, Merkle openings, in-circuit vs skip
- [Neighbor experiments (single scenario)](/Users/miha/.cursor/projects/Users-miha-projects-fightfake-projects-eva-miha/canvases/neighbor-experiments.canvas.tsx) — Chebyshev halo, I vs P, leak semantics, measured 8-frame bars
- [Experiment matrix (CSV sweep)](/Users/miha/.cursor/projects/Users-miha-projects-fightfake-projects-eva-miha/canvases/experiment-matrix.canvas.tsx) — four box/GOP scenarios, bar charts from synthetic runs
| [`docs/720p-edit-walkthrough.md`](docs/720p-edit-walkthrough.md) | **Start-to-finish:** source clip, redact edit, exact gadget/halo MBs, decode smear without pixel edit |
| [`720p-edit-walkthrough.canvas.tsx`](/Users/miha/.cursor/projects/Users-miha-projects-fightfake-projects-eva-miha/canvases/720p-edit-walkthrough.canvas.tsx) | Interactive grids: pixel edit, island, decode smear (GOP=1/8) |

---

## Terminology

| Symbol | Meaning |
|--------|---------|
| **CIF** | Common Intermediate Format: **352×288** — used in the Eva paper dataset (Foreman), **not** the neighbor-experiment default |
| **HD default** | **1280×720 × 30 frames**, **80×45 = 3600** MBs/frame; fixture `docs/fixtures/sample_720p_30f.yuv` |
| **Gadget** | Macroblocks whose pixels are painted in the edit box on redact frames |
| **Halo** | MBs within Chebyshev distance `halo_mbs` of any gadget MB (same frames), excluding gadgets |
| **Island I** | Gadget ∪ halo on redact frames; **only I** is re-encoded in the Nova circuit |
| **Skip J** | All other MB×frame slots; syntax copied from capture, not proved in Nova |
| **Pre / post** | Frames before `frame_start` / from `frame_end` onward (always skip in glue) |
| **Leak** | Decoded luma differs outside **I** after an operation that should be local (pixel edit, intra re-encode, or GOP reference smear) |

Chebyshev halo: for gadget at grid `(cx, cy)`, halo MBs satisfy `max(|c−cx|, |r−cy|) ≤ halo_mbs` and are not gadgets.

---

## Code map

| Path | Role |
|------|------|
| `video/src/island.rs` | `IslandSpec`, `island_counts`, `mb_role`, halo geometry |
| `video/src/merkle.rs` | SHA3-256 pixel/syntax Merkle trees, `MerkleProof`, leaf hashing |
| `video/src/neighbor_impact.rs` | `compare_yuv_to_island`, `ImpactReport` (gadget/halo/skip/pre/post) |
| `video/src/neighbor_experiment.rs` | Shared runner: synthetic YUV, redact, ffmpeg x264 compare, CSV rows |
| `video/examples/island_report.rs` | Geometry-only island calculator (no encode) |
| `video/examples/neighbor_reencode.rs` | Single-scenario pixel + optional GOP=1 / GOP=8 compare |
| `video/examples/experiment_matrix.rs` | Fixed scenario sweep → CSV |
| `video/examples/mb_grid_export.rs` | Per-MB grid JSON for canvas (pixel / decode / optional syntax) |
| `video/src/mb_grid.rs` | `build_mb_grid_report`, `compare_yuv_per_mb`, JSON export |

---

## Glue protocol (design summary)

### Capture

1. Merkle-commit all macroblock **picture** leaves → root `R_pix`.
2. Merkle-commit all **syntax** leaves → root `R_syn`.
3. Sign `σ = Sign(R_pix ‖ R_syn)`.

No island at capture time.

### Prove (editor)

1. Define island **I** from edit box + Chebyshev halo (+ future P-MV closure).
2. For each index in **I**: Merkle-open original picture leaf `P_i` → `R_pix`; redact pixels; forward-encode in circuit → `S′_i`; Griffin hashes `h1_I`, `h2_I`.
3. For **J** (skip): copy captured syntax `S_j` verbatim; not in Nova.

### Verify

1. **V0**: check signature on `(R_pix, R_syn)` — no YUV needed.
2. Per skip slot **j**: syntax Merkle open to `R_syn`; picture Merkle open of **published original** `P_j` to `R_pix` (not decoded paint).
3. Island: verify SNARK + file tile `T_i` matches `h2_I`.

**Known gaps** (see glue-protocol canvas):

- Skip **S↔P** (`Encode(P)=S`) is not proved in Nova for skip tiles — two Merkle opens ≠ encode equality.
- ffmpeg full re-encode contamination is an **upper bound** on splice leak, not exact MV closure.
- No whole-file MP4 hash; integrity is per-tile over `I ∪ J`.

---

## Running experiments

Requires **ffmpeg** with `libx264` for re-encode columns. Pixel-only columns work without ffmpeg.

### Single scenario

```bash
# Synthetic CIF gradient, box (96,80,176,144), frames [2,4), halo 1
cargo run --release -p video --example neighbor_reencode -- \
  352 288 8 96 80 176 144 2 4

# Real YUV (raw yuv420p, frame size must match)
cargo run --release -p video --example neighbor_reencode -- \
  352 288 30 96 80 176 144 14 16 --yuv /path/to/clip.yuv --gop 8
```

### Scenario matrix (CSV)

```bash
# Default: 1280×720 × 30 frames (loads docs/fixtures/sample_720p_30f.yuv if present)
./scripts/fetch_fixture_720p.sh   # once, ~40 MB gitignored YUV

cargo run --release -p video --example experiment_matrix -- \
  --out docs/results/neighbor-matrix-720p.csv

# Your clip
cargo run --release -p video --example experiment_matrix -- \
  --yuv /path/to/clip.yuv --width 1920 --height 1080 --frames 60 --out matrix.csv

# Legacy CIF / Foreman (Eva dataset resolution — low, not representative)
cargo run --release -p video --example experiment_matrix -- \
  --yuv /path/to/foreman_cif.yuv --width 352 --height 288 --frames 300 --out foreman.csv
```

### Geometry only

```bash
cargo run --release -p video --example island_report -- \
  352 288 8 96 80 176 144 2 4 1
```

### Macroblock grid (visualization JSON)

```bash
# Default 720p (after ./scripts/fetch_fixture_720p.sh)
cargo run --release -p video --example mb_grid_export -- \
  --gop 8 --out docs/results/mb-grid-720p-gop8.json
```

Open [macroblock-grid.canvas.tsx](/Users/miha/.cursor/projects/Users-miha-projects-fightfake-projects-eva-miha/canvases/macroblock-grid.canvas.tsx) beside chat. Layers:

| Layer | Meaning |
|-------|---------|
| **Pixel edit** | Red = gadget pixels painted; gray = unedited |
| **Island I/J** | Gadget + halo vs skip (geometry) |
| **Decode diff** | Yellow = decoded luma changed after two ffmpeg encodes (not syntax) |
| **Syntax** | Purple = must re-encode in circuit; dashed = copy syntax (until JM dumps loaded) |
| **Overlay** | Red fill + yellow dot = pixel edit + decode leak on same tile |

---

## CSV columns (`experiment_matrix`)

| Column | Meaning |
|--------|---------|
| `label` | Scenario name |
| `width`, `height`, `frames` | Clip geometry |
| `box_*`, `frame_start`, `frame_end`, `halo` | `IslandSpec` |
| `gadget_mb`, `halo_mb`, `island_mb`, `total_mb`, `island_pct` | Geometric counts |
| `pixel_outside_i` | MBs outside **I** with ΔY>0 after **pixel redact only** (expect **0**) |
| `intra_outside_i_d0` | Outside **I** after independent **GOP=1** encode/decode (ΔY>0) |
| `intra_outside_i_d2` | Same, ΔY≥2 |
| `intra_halo_d0` | Halo MBs changed under GOP=1 (quantization smear) |
| `gop` | GOP size used for temporal scenario (8 or 1) |
| `gop_outside_i_d0` | Outside **I** with ΔY>0 after GOP=`gop` re-encode |
| `gop_post_d0` | **Post-window** MBs changed (temporal reference leak) |
| `gop_outside_i_d2`, `gop_post_d2` | Same at ΔY≥2 |

**Interpretation:**

- **`pixel_outside_i = 0`** → native redact is strictly local; glue copy-outside-**I** is safe at pixel layer.
- **`intra_outside_i_* > 0`** → full re-encode smears outside **I** even with GOP=1 (~18 MBs on center box); irrelevant to splice-if-you-copy-syntax, but shows encoder non-locality.
- **`gop_post_*` large** → P-frame reference chain pollutes **post** frames; motivates skip-copy for **J** and bounded **I**.

---

## Measured results (Sep 2026)

Encoder: ffmpeg `libx264`, `-preset ultrafast`, `-qp 23`, `-bf 0`.

### 720p sample clip, 30 frames, redact `[14,16)`, halo 1

Center box **640×360 px** at `(320,180)` — ~half frame.

| Scenario | \|I\| / total | pixel out | intra out (d0) | GOP8 post (d0) |
|----------|-------------|-----------|----------------|----------------|
| `hd_center_box` | 2100 / 108000 (**1.9%**) | 0 | 81 | 0 |
| `small_box` | 792 / 108000 (**0.7%**) | 0 | 72 | 0 |
| `wide_box` | 3472 / 108000 (**3.2%**) | 0 | 143 | 0 |
| `hd_center_gop1` | 1.9% | 0 | 81 | — |

Committed CSV: `docs/results/neighbor-matrix-720p.csv`. Macroblock grid canvas embeds frames 0, 14, 15, 29.

**Note:** `gop_post = 0` here because the redact window is mid-clip (no post frames in the same GOP window as the edit). CIF 8-frame runs showed large post leak because redact `[2,4)` leaves many P frames after.

### Legacy: synthetic CIF 352×288 (8 frames, redact `[2,4)`)

Kept in `docs/results/neighbor-matrix-synthetic-*.csv` for comparison only — **9× fewer pixels per frame** than 720p.

---

## What is *not* measured yet

1. **Your own footage** — pass `--yuv`; Foreman CIF is Eva-benchmark resolution, not phone/web video.
2. **Syntax-level I** — paired JM encoder dumps → `syntax_mb_changed` for true encode footprint.
3. **P-MV closure** — expand **I** beyond Chebyshev halo for inter prediction.
4. **In-circuit Nova + verify harness** — island-only prove, capture sign artifact, CABAC glue.

---

## Tests

```bash
cargo test -p video island merkle neighbor
```

Covers halo geometry, Merkle verify, pixel redact locality, and matrix pixel-outside-I=0.

---

## Related docs

- `docs/island-only-proving.md` — why island-only is hard, measured sweep, open gaps, next steps
- `docs/capture-signing-and-ingest.md` — capture signing
- `docs/edit-only/README.md` — edit-only folding experiments
