# Lossless encoding in Eva

Eva's default path proves **edit + H.264 forward encode** and binds the output to
predictors and quantized coefficients from JM dumps (~1.4M constraints/step at 256 blocks).

**Lossless encoding** means skipping that H.264 encode proof: input and output are both
**macroblock YUV pixels** (spatial domain), and the proof only attests the edit transform.

| | Lossy (default) | Lossless (this branch) |
|---|---|---|
| Input | Macroblock YUV from JM dumps | Same |
| Circuit proves | Edit + DCT/quant encode | Edit only |
| h1 | Hash of **original** pixels | Same |
| h2 | Hash of preds + **coeffs** + QP + edit cfg | Hash of **edited pixels** + edit cfg |
| Witnesses needed | orig + preds + coeffs + encode cfg | orig pixels + edit cfg only |

This is **not** "raw `.yuv` file vs H.264 container" — both paths use the same
macroblock-shaped pixel witnesses. Lossless means no lossy re-quantization in the proof.

## Code

| Path | Purpose |
|------|---------|
| `video/src/edit_only.rs` | `EditOnlyCircuit` — lossless IVC step |
| `video/examples/edit_bright_only.rs` | Nova prove + verify smoke test |
| `video/examples/edit_lossless_decider.rs` | Full pipeline: Nova + Groth16 decider |
| `video/examples/hash_verifier_lossless.rs` | Native h2 over edited pixels (off-chain check) |
| `video/examples/yuv_to_macroblocks.rs` | **mp4/ffmpeg YUV → Eva macroblock files** |
| `video/examples/macroblocks_to_yuv.rs` | **Macroblocks → YUV for ffplay** (optional `BRIGHTNESS=`) |

Native helpers: `hash_orig_macroblock`, `hash_edited_macroblock`, `yuv420_to_macroblocks`,
`macroblocks_to_yuv420` (re-exported from `video`).

## Quick start: your own video

**Do not** put `.mp4` in `data_parsed/`. Eva only reads macroblock dumps.

| Step | Where / what |
|------|----------------|
| 1. Your file | `my_clip.mp4` anywhere (Desktop, `~/videos`, …) |
| 2. ffmpeg YUV | `my_clip.yuv` (planar `yuv420p`; width & height **÷ 16**) |
| 3. Eva input | `data_parsed/<name>/orig_y_enc`, `orig_u_enc`, `orig_v_enc` via `yuv_to_macroblocks` |
| 4. **Playable edited video** | `macroblocks_to_yuv` + `BRIGHTNESS=` → `.yuv` or `.mp4` (see below) |
| 5. Prove (optional) | `export DATA_PATH=.../data_parsed` then `VIDEO=<name> cargo run … edit_bright_only` |

Steps 4 and 5 are **independent** — you do not need to run the proof first to export a video.

`DATA_PATH` is **compile-time** — set it in the shell before `cargo run` (rebuild if you change it).
`VIDEO` selects the subfolder under `DATA_PATH` (default `foreman`).

```bash
cd /path/to/eva-miha
mkdir -p data_parsed/my_clip

# mp4 → YUV
ffmpeg -i ~/videos/my_clip.mp4 -vf scale=352:288 -pix_fmt yuv420p -frames:v 30 my_clip.yuv

# YUV → Eva macroblocks (only originals — no edited file exists yet)
cargo run --release -p video --example yuv_to_macroblocks -- \
  my_clip.yuv data_parsed/my_clip 352 288 30

# Playable EDITED video (separate from proving — applies brightness off-chain)
BRIGHTNESS=416 cargo run --release -p video --example macroblocks_to_yuv -- \
  data_parsed/my_clip my_clip_edited.yuv 352 288 30
ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 my_clip_edited.yuv
# Or share as mp4:
ffmpeg -f rawvideo -pix_fmt yuv420p -s 352x288 -r 30 -i my_clip_edited.yuv \
  -c:v libx264 -pix_fmt yuv420p my_clip_edited.mp4

# Cryptographic proof (does not write any video file)
export DATA_PATH="$(pwd)/data_parsed"
VIDEO=my_clip QUICK=1 cargo run --release -p video --example edit_bright_only
```

## Prove vs playable edited video

Running `edit_bright_only` (or any prover) **does not create a video file**. It only:

- checks the edit gadget + hashes in zero knowledge
- prints an IVC final state (field elements)
- optionally verifies the Nova proof in memory

There is **no edited video on disk** after proving. Eva never writes `edited.mp4`.

To **see** the edited result, run **`macroblocks_to_yuv`** on the same `orig_*_enc` files.
It applies the same transform as the circuit (`Brightness::edit_native` when `BRIGHTNESS=416`)
and writes a normal planar YUV file you can play or convert to mp4.

```
data_parsed/my_clip/orig_*_enc   (original pixels only)
        │
        ├─► macroblocks_to_yuv + BRIGHTNESS=416  ──►  my_clip_edited.yuv / .mp4   ← watch this
        │
        └─► edit_bright_only                       ──►  proof + hashes (no video file)
```

Use the **same** `BRIGHTNESS` value as `BrightnessCfg(...)` in the proof example (`416` today).
Original (no edit): run `macroblocks_to_yuv` **without** `BRIGHTNESS`.

### Export edited video (copy-paste)

Replace `my_clip`, `352`, `288`, `30` with your folder name and dimensions.

```bash
# Edited (brightness — matches edit_bright_only)
BRIGHTNESS=416 cargo run --release -p video --example macroblocks_to_yuv -- \
  data_parsed/my_clip my_clip_edited.yuv 352 288 30

# Play
ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 my_clip_edited.yuv

# Or mp4
ffmpeg -f rawvideo -pix_fmt yuv420p -s 352x288 -r 30 -i my_clip_edited.yuv \
  -c:v libx264 -pix_fmt yuv420p my_clip_edited.mp4
```

## What “native edit” means (and what it is not)

**Native edits exist** — they are fixed transforms in `video/src/edit/constraints.rs`
(brightness, crop, invert, grayscale, mask). The proof runs `edit_circuit` on original
pixels + a **config**; you never upload a separately edited video as the witness.

| | Native Eva edit | Runway / Premiere / ffmpeg filters |
|--|-----------------|-------------------------------------|
| Where it runs | Inside the zk circuit (`EditGadget`) | Outside Eva |
| Can be proved? | ✅ if gadget is implemented | ❌ |
| Separate “edit app”? | **No** — config is set in Rust examples | Yes (those tools) |
| Preview off-chain | `edit_native` / `BRIGHTNESS=` in `macroblocks_to_yuv` | Those tools’ export |

So: there is **no Eva video editor**. You choose a gadget + config in code (e.g.
`BrightnessCfg(416)`), the circuit applies it, and `h2` binds the result. For a demo,
use Runway/ffmpeg only to obtain the **original** clip; the **proved** change is the Eva gadget.

**Lossless tooling today:** only **brightness** is wired end-to-end (`edit_bright_only`,
`BRIGHTNESS=` preview). Other gadgets work in the **lossy** `edit_*_decider` examples;
lossless copies (`edit_crop_only`, etc.) still need to be added (see [Extending](#extending)).

## Native Eva edits (what you *can* prove)

The lossless path does **not** witness an edited video file. It witnesses:

1. **Original** macroblock YUV (`orig_*_enc`)
2. An **edit config** per macroblock (gadget-specific)

The circuit runs `edit_circuit` in zero-knowledge and binds the **resulting pixels** in `h2`.
Preview off-chain with the matching `edit_native` (e.g. `BRIGHTNESS=416` in `macroblocks_to_yuv`).

| Gadget (`EditGadget`) | Config | Effect | Lossless examples today | Lossy decider (encode + edit) |
|----------------------|--------|--------|-------------------------|-------------------------------|
| `Brightness` | `BrightnessCfg(scale)` — e.g. `416` ≈ ×1.62 luma | Scale Y; U/V unchanged | `edit_bright_only`, `edit_lossless_decider`, `hash_verifier_lossless` | `edit_bright_decider` |
| `Removing` | `RemovingCfg(keep)` — per macroblock bool | Zero macroblock if outside crop | *copy from `edit_crop_decider`* | `edit_crop_decider`, `edit_cut_decider` |
| `InvertColor` | `()` | `255 − pixel` on Y/U/V | *not wired yet* | `edit_inv_decider` |
| `Grayscale` | `()` | Keep Y; U/V → 128 | *not wired yet* | `edit_gray_decider` |
| `Masking` | `MaskCfg(...)` | Per-pixel mask | *not wired yet* | `edit_mask_decider` |
| `NoOp` | `()` | Identity (no pixel change) | *not wired yet* | `edit_noop_decider` |

Implementation: `video/src/edit/constraints.rs`. Generic circuit: `EditOnlyCircuit<Fr, YourGadget>`.

**Demo recipe:** any video source (Runway, phone, `foreman`) → `yuv_to_macroblocks` → pick a
**native** gadget above → prove with `EditOnlyCircuit`. Runway is only a convenient way to
obtain the **original** clip; the proved transform is always an Eva gadget.

## Custom video (e.g. Runway export)

Eva cannot prove arbitrary Runway/NLE edits — use a **native gadget** from the table above.
A practical demo: Runway (or ffmpeg) for the **source clip**, then prove **brightness** or **crop**.

```bash
# 1. Export from Runway → mp4, then raw YUV (size must be multiple of 16)
ffmpeg -i runway.mp4 -vf scale=352:288 -pix_fmt yuv420p -frames:v 30 runway.yuv

# 2. Pack into Eva macroblock files
cargo run --release -p video --example yuv_to_macroblocks -- \
  runway.yuv ./data_parsed/runway_demo 352 288 30

# 3. Watch original / edited preview
cargo run --release -p video --example macroblocks_to_yuv -- \
  ./data_parsed/runway_demo runway_orig.yuv 352 288 30
BRIGHTNESS=416 cargo run --release -p video --example macroblocks_to_yuv -- \
  ./data_parsed/runway_demo runway_bright.yuv 352 288 30
ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 runway_bright.yuv

# 4. Prove (lossless, no encode) — set VIDEO to your folder name
export DATA_PATH=/path/to/data_parsed
VIDEO=runway_demo QUICK=1 cargo run --release -p video --example edit_bright_only
```

### What `yuv_to_macroblocks` does

1. Reads **planar YUV 4:2:0** (Y plane, then U, then V — ffmpeg `yuv420p` layout).
2. For each 16×16 luma region, copies 256 bytes into `orig_y_enc` (row-major pixels).
3. For each 8×8 chroma region, copies into `orig_u_enc` / `orig_v_enc`.
4. Macroblock order matches Eva: left→right, top→bottom (same as `edit_crop_decider`).

### What `macroblocks_to_yuv` does

The inverse: stitches macroblocks back into a playable `.yuv` file. With `BRIGHTNESS=<u16>`,
applies the same luma scaling as `BrightnessCfg` in the proof so you can preview the
**proved** edit (not the Runway effect).

## End-to-end flow

```
hash_recorder (foreman/)     →  h1 chain over original pixels
EditOnlyCircuit Nova proof   →  IVC state (h1, h2) in-circuit
hash_verifier_lossless       →  native h2 over edited pixels (should match proof z[1])
edit_lossless_decider        →  Groth16 decider + signature on h1
```

## Run

```bash
export DATA_PATH=/path/to/data_parsed

# Fast Nova smoke (4 blocks/step, 2 steps)
QUICK=1 cargo run --release -p video --example edit_bright_only

# Native h2 for brightness edit (compare to proof final state[1])
cargo run --release -p video --example hash_verifier_lossless

# Full lossless pipeline with decider (slow: Groth16 setup)
QUICK=1 cargo run --release -p video --example edit_lossless_decider

# Unit tests (no dataset)
cargo test -p video edit_only --release
```

## Extending

`EditOnlyCircuit<F, E>` is generic over `EditGadget`. To add a lossless example for crop:

1. Copy `edit_bright_only.rs` → `edit_crop_only.rs`
2. Set `type Op = Removing` and build `RemovingCfg` per macroblock (see `edit_crop_decider`)
3. Preview with `Removing::edit_native` in a small script, or extend `macroblocks_to_yuv`

Lossy-path references: `edit_*_decider` and `hash_verifier_*` for each gadget.

## Branch

`edit-only-proof`
