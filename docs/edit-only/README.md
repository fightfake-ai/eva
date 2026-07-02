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
