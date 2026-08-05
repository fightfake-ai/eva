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
| `video/examples/hash_verifier_lossless.rs` | Native-reference h2 over edited pixels (compare to proof) |
| `video/examples/yuv_to_macroblocks.rs` | **mp4/ffmpeg YUV → Eva macroblock files** |
| `video/examples/image_to_macroblocks.rs` | **PNG/JPEG/… → Eva macroblock files** (`num_frames=1`) |
| `video/examples/native_edit_export_yuv.rs` | **Optional native edit + export to planar YUV** (`BRIGHTNESS=`) |
| `video/src/rgb_yuv.rs` | RGB8 → YUV420p helper for still ingest |

Native helpers: `hash_orig_macroblock`, `hash_edited_macroblock`, `yuv420_to_macroblocks`,
`macroblocks_to_yuv420` (phase 2: export), `native_brightness_edit_macroblocks` (phase 1: edit).

### Naming: ingest, edit, export

| Tool / function | Phase | What it does |
|-----------------|-------|----------------|
| `yuv_to_macroblocks` | Ingest | Planar YUV → `orig_*_enc` macroblock files |
| `image_to_macroblocks` | Ingest | Still image → `orig_*_enc` (`num_frames=1`) |
| `native_brightness_edit_macroblocks` | **1 — native edit** | `edit/native_macroblocks.rs` — `edit_native` per macroblock |
| `macroblocks_to_yuv420` | **2 — export** | `macroblock_yuv.rs` — macroblock bytes → planar YUV |
| `native_edit_export_yuv` | 1 + 2 (example) | Optional `BRIGHTNESS=` then writes `.yuv` |

## Quick start: still image

A still is just **one frame** of macroblocks (`num_frames = 1`). No ffmpeg required for ingest.

| Step | What |
|------|------|
| 1. Your file | `photo.png` / `.jpg` / … (any size ≥ 16×16; cropped to ×16) |
| 2. Eva macroblocks | `image_to_macroblocks` → `data_parsed/<name>/orig_*_enc` |
| 3. Prove (optional) | `edit_bright_only` **or** toolkit `prove-edit --input photo.png` |

```bash
cd /path/to/eva-miha
mkdir -p data_parsed/photo

cargo run --release -p video --example image_to_macroblocks -- \
  ~/pictures/photo.png data_parsed/photo

# Cryptographic proof (does not write an image file)
export DATA_PATH="$(pwd)/data_parsed"
VIDEO=photo QUICK=1 cargo run --release -p video --example edit_bright_only

# Or full toolkit workflow (decode + edit + proof + C2PA, 1-frame MP4 out):
# cd ../fightfake-toolkit && cargo run --release -p fightfake-cli -- \
#   prove-edit --input ~/pictures/photo.png --gadget brightness --out-dir out/
```

For **redact** on a still via toolkit, `--redact-frame-end` defaults to `1` when the
input is an image; pass `--redact-width` / `--redact-height` (and optional x/y).

## Quick start: your own video

**Do not** put `.mp4` in `data_parsed/`. Eva only reads macroblock dumps. MP4 → YUV → macroblocks
is trusted ingest (not proved); see [`capture-signing-and-ingest.md`](../capture-signing-and-ingest.md).

| Step | Where / what |
|------|----------------|
| 1. Your file | `my_clip.mp4` anywhere (Desktop, `~/videos`, …) |
| 2. ffmpeg YUV | `my_clip.yuv` (planar `yuv420p`; width & height **÷ 16**) |
| 3. Eva input | `data_parsed/<name>/orig_y_enc`, `orig_u_enc`, `orig_v_enc` via `yuv_to_macroblocks` |
| 4. **Playable edited video** | `native_edit_export_yuv` + `BRIGHTNESS=` → `.yuv` or `.mp4` (see below) |
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

# Playable EDITED video (uses edit_native reference — not the in-circuit prover)
BRIGHTNESS=416 cargo run --release -p video --example native_edit_export_yuv -- \
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

To **see** the edited result, run **`native_edit_export_yuv`** on the same `orig_*_enc` files.
It applies the same transform as the circuit (`Brightness::edit_native` when `BRIGHTNESS=416`)
and writes a normal planar YUV file you can play or convert to mp4.

```
data_parsed/my_clip/orig_*_enc   (original pixels only)
        │
        ├─► native_edit_export_yuv + BRIGHTNESS=416  ──►  my_clip_edited.yuv / .mp4   ← watch this
        │
        └─► edit_bright_only                       ──►  proof + hashes (no video file)
```

Use the **same** `BRIGHTNESS` value as `BrightnessCfg(...)` in the proof example (`416` today).
Original (no edit): run `native_edit_export_yuv` **without** `BRIGHTNESS`.

### Export edited video (copy-paste)

Replace `my_clip`, `352`, `288`, `30` with your folder name and dimensions.

```bash
# Edited (brightness — matches edit_bright_only)
BRIGHTNESS=416 cargo run --release -p video --example native_edit_export_yuv -- \
  data_parsed/my_clip my_clip_edited.yuv 352 288 30

# Play
ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 my_clip_edited.yuv

# Or mp4
ffmpeg -f rawvideo -pix_fmt yuv420p -s 352x288 -r 30 -i my_clip_edited.yuv \
  -c:v libx264 -pix_fmt yuv420p my_clip_edited.mp4
```

### Video length (why only ~1 second?)

Raw `.yuv` has **no timestamps** — duration = `num_frames ÷ playback_fps`.

| Your pack step | Result |
|----------------|--------|
| `yuv_to_macroblocks … 352 288 30` | **30 frames** in `orig_*_enc` |
| `ffplay … -s 352x288` (default ~25 fps) | ~1.2 s |
| `ffmpeg … -r 30 -i …` when making mp4 | **exactly 1.0 s** at 30 fps |

For a longer clip, pack **more frames** when converting from mp4:

```bash
# 10 seconds at 30 fps → 300 frames
ffmpeg -i my_clip.mp4 -vf scale=352:288 -pix_fmt yuv420p -frames:v 300 my_clip.yuv
cargo run --release -p video --example yuv_to_macroblocks -- \
  my_clip.yuv data_parsed/my_clip 352 288 300
# Use 300 as the last argument to native_edit_export_yuv as well
```

Check frame count: `orig_y_enc` bytes ÷ 256 ÷ (width/16 ÷ height/16 macroblocks per frame).
For 352×288 that's 396 macroblocks/frame (`orig_y_enc` size ÷ 256 ÷ 396).

**Note:** Lossless folders only need `orig_*_enc`. Do **not** use `parse_prover_data` paths
for custom clips — `edit_bright_only` uses `parse_orig_blocks` (encode files not required).

## Native Eva edits (supported gadgets)

The lossless path does **not** witness an edited video file. It witnesses:

1. **Original** macroblock YUV (`orig_*_enc`)
2. An **edit config** per macroblock (gadget-specific)

The circuit runs `edit_circuit` in zero-knowledge and binds the **resulting pixels** in `h2`.
To preview pixels without proving, use `edit_native` (e.g. `BRIGHTNESS=416` in `native_edit_export_yuv`).

**Terminology:** each `EditGadget` has two implementations of the same transform:

| | `edit_native` | `edit_circuit` |
|--|---------------|----------------|
| Where | Plain Rust (reference) | R1CS constraints inside `EditOnlyCircuit` |
| Used for | Video export, `hash_edited_macroblock`, tests | Nova / IVC proof |
| Proves anything? | No (but must match `edit_circuit`) | Yes |

There is no separate Eva video editor — you set gadget + config in code (e.g. `BrightnessCfg(416)`).

**Lossless tooling today:** only **brightness** is wired end-to-end (`edit_bright_only`,
`BRIGHTNESS=` in `native_edit_export_yuv`). Other gadgets have **lossy** `edit_*_decider` examples;
lossless copies (`edit_crop_only`, etc.) still need to be added (see [Extending](#extending)).

| Gadget (`EditGadget`) | Config | Effect | Lossless examples today | Lossy decider (encode + edit) |
|----------------------|--------|--------|-------------------------|-------------------------------|
| `Brightness` | `BrightnessCfg(scale)` — e.g. `416` ≈ ×1.62 luma | Scale Y; U/V unchanged | `edit_bright_only`, `edit_lossless_decider`, `hash_verifier_lossless` | `edit_bright_decider` |
| `Removing` | `RemovingCfg(keep)` — per macroblock bool | Zero macroblock if outside crop | *copy from `edit_crop_decider`* | `edit_crop_decider`, `edit_cut_decider` |
| `InvertColor` | `()` | `255 − pixel` on Y/U/V | *not wired yet* | `edit_inv_decider` |
| `Grayscale` | `()` | Keep Y; U/V → 128 | *not wired yet* | `edit_gray_decider` |
| `Masking` | `MaskCfg(...)` | Per-pixel mask | *not wired yet* | `edit_mask_decider` |
| `NoOp` | `()` | Identity (no pixel change) | *not wired yet* | `edit_noop_decider` |

Implementation: `video/src/edit/constraints.rs`. Generic circuit: `EditOnlyCircuit<Fr, YourGadget>`.

## Brightness: what code actually runs

Each `EditGadget` exposes **`edit_native`** (reference) and **`edit_circuit`** (constrained).
For brightness, both implement the same luma scaling; the proof path uses `edit_circuit` plus
Griffin hashes and Nova folding.

### The edit itself (shared math)

Defined in `video/src/edit/constraints.rs` on `impl EditGadget for Brightness`:

| | Function | What it does |
|--|----------|----------------|
| **Native (reference)** | `Brightness::edit_native` | For each Y pixel: `Y' = min(255, Y × scale ÷ 256)`. U and V copied unchanged. |
| **In-circuit** | `Brightness::edit_circuit` | Same relation, as R1CS: proves `pixel × scale` decomposes correctly and output is `min(255, …)`. |

Config type: `BrightnessCfg(scale: u16)` — e.g. `416` means multiply luma by `416/256 ≈ 1.62`.
In the proof examples this is set in `edit_bright_only.rs` as `let brightness = BrightnessCfg(416)`.

### A) Playable edited video — `native_edit_export_yuv` + `BRIGHTNESS=416`

Uses **`edit_native`** only (no R1CS, no proof). Example command:

```bash
BRIGHTNESS=416 cargo run --release -p video --example native_edit_export_yuv -- \
  data_parsed/my_clip my_clip_edited.yuv 352 288 30
```

#### Where `BRIGHTNESS=416` is read

| Step | File | What runs |
|------|------|-----------|
| 1 | `video/examples/native_edit_export_yuv.rs` | `main()` parses CLI args (`data_parsed/my_clip`, output path, 352, 288, 30) |
| 2 | same, ~line 64 | `env::var("BRIGHTNESS")` → parse as `u16` → `Some(416)` |
| 3 | same, ~line 66 | `read_macroblock_dir(&input_dir)` loads `orig_y_enc`, `orig_u_enc`, `orig_v_enc` |
| 4 | same, ~line 102 | **Phase 1:** `native_brightness_edit_macroblocks(..., 416)` → edited byte streams |
| 5 | same, ~line 118 | **Phase 2:** `macroblocks_to_yuv420(&edited_y, …)` → planar YUV |
| 6 | `video/src/edit/native_macroblocks.rs` | Phase 1 calls `Brightness::edit_native` per macroblock |
| 7 | `video/src/edit/constraints.rs` ~line 287 | `edit_native`: `min(255, Y × 416 ÷ 256)` on luma |
| 8 | `macroblock_yuv.rs` | Phase 2: `insert_y_block` / `insert_uv_block` stitch planes |
| 9 | `native_edit_export_yuv.rs` ~line 130 | `fs::write(output, yuv)` → `my_clip_edited.yuv` |

If `BRIGHTNESS` is unset, only phase 2 runs on the original `orig_*_enc` bytes.

#### Call tree

```
native_edit_export_yuv::main()                    video/examples/native_edit_export_yuv.rs
  ├─ env::var("BRIGHTNESS")  →  Option<u16>   (416)
  ├─ read_macroblock_dir()                    video/src/macroblock_yuv.rs
  │    └─ fs::read orig_y_enc, orig_u_enc, orig_v_enc
  ├─ if BRIGHTNESS=416:
  │    native_brightness_edit_macroblocks(..., 416)   phase 1 — edit/native_macroblocks.rs
  │    macroblocks_to_yuv420(edited_y, edited_u, edited_v, …)   phase 2
  ├─ else:
  │    macroblocks_to_yuv420(orig_*, …)                phase 2 only
  └─ fs::write(my_clip_edited.yuv)
```

No zk code runs on this path — no `EditOnlyCircuit`, no `edit_circuit`, no Nova.

#### Key snippets

`BRIGHTNESS` env → function argument (`native_edit_export_yuv.rs`):

```rust
let brightness = env::var("BRIGHTNESS").ok().and_then(|s| s.parse().ok());
let yuv = match brightness {
    Some(scale) => {
        let (y, u, v) = native_brightness_edit_macroblocks(&orig_y, &orig_u, &orig_v, width, height, num_frames, scale)?;
        macroblocks_to_yuv420(&y, &u, &v, width, height, num_frames)?
    }
    None => macroblocks_to_yuv420(&orig_y, &orig_u, &orig_v, width, height, num_frames)?,
};
```

Phase 1 (`edit/native_macroblocks.rs`):

```rust
let (y, u, v) = Brightness::edit_native(&y, &u, &v, &BrightnessCfg(brightness_scale));
// writes edited bytes to out_y / out_u / out_v
```

The actual pixel transform (`constraints.rs`, `impl EditGadget for Brightness`):

```rust
y.iter()
    .map(|&v| min(255, (v as u64 * cfg.0 as u64) >> 8) as u8)  // cfg.0 == 416
    .collect()
// u.clone(), v.clone() — chroma unchanged
```

Compare to the proof path: `edit_bright_only.rs` hard-codes `let brightness = BrightnessCfg(416)` and passes it into `EditOnlyCircuit`, which calls `Brightness::edit_circuit` instead of `edit_native`.

### B) Cryptographic proof — `edit_bright_only`

Uses **`edit_circuit`** inside the IVC step:

```
edit_bright_only (example)
  └─ parse_orig_blocks()               video/src/macroblock_yuv.rs
  └─ Nova::preprocess / prove_step
       └─ EditOnlyCircuit               video/src/edit_only.rs
            └─ process_macroblock() per 16×16 block:
                 1. commit orig Y/U/V     MatrixVar::new_committed
                 2. witness config        BrightnessCfgVar (scale 416)
                 3. Brightness::edit_circuit()  → edited Y/U/V vars
                 4. h1 = Griffin hash of **original** pixels
                 5. h2 = Griffin hash of **edited** pixels + compactify(config)
            └─ fold_step_hashes()        chain h1/h2 across macroblocks → IVC state
```

Witness input struct: `EditOnlyExternalInputs { blocks, edit_configs }` — **original pixels only**,
plus one `BrightnessCfg` per macroblock. No edited video file is read.

Native reference for hashing (tests / `hash_verifier_lossless`): `hash_edited_macroblock` in
`edit_only.rs` calls `E::edit_native` then Griffin-hash — must match step `h2` partial hashes.

### C) How config enters the hash (`h2`)

`BrightnessCfg::compactify` appends the scale as one field element to the `h2` hash input
(alongside edited pixel bytes). Crop/mask gadgets pack more data; brightness only adds `scale`.

### Quick map: command → code

| You run | Implementation | Output |
|---------|----------------|--------|
| `BRIGHTNESS=416 … native_edit_export_yuv` | `edit_native` (reference) | `.yuv` / `.mp4` file |
| `VIDEO=… edit_bright_only` | `edit_circuit` (+ hashes, Nova) | proof; `IVC final state` |
| `hash_verifier_lossless` | `hash_edited_macroblock` → `edit_native` | printed `h2` (check vs proof) |

## Tool reference

### What `yuv_to_macroblocks` does

1. Reads **planar YUV 4:2:0** (Y plane, then U, then V — ffmpeg `yuv420p` layout).
2. For each 16×16 luma region, copies 256 bytes into `orig_y_enc` (row-major pixels).
3. For each 8×8 chroma region, copies into `orig_u_enc` / `orig_v_enc`.
4. Macroblock order matches Eva: left→right, top→bottom (same as `edit_crop_decider`).

### What `native_edit_export_yuv` does

Two phases (phase 1 is optional):

1. **Native edit** — if `BRIGHTNESS=<u16>` is set, `native_brightness_edit_macroblocks` runs
   `Brightness::edit_native` on every macroblock.
2. **Export** — `macroblocks_to_yuv420` stitches macroblocks into planar YUV 4:2:0 and writes the file.

Without `BRIGHTNESS`, only step 2 runs via `macroblocks_to_yuv420`. This is the export
counterpart to `yuv_to_macroblocks` (ingest). See
[A) Playable edited video](#a-playable-edited-video--native_edit_export_yuv--brightness416).

## Trust boundary

Eva proofs start at `orig_*_enc` macroblock witnesses. Camera capture, MP4 decode, YUV packing,
and export are outside the circuit unless you design otherwise. See
[`docs/capture-signing-and-ingest.md`](../capture-signing-and-ingest.md) for what signatures bind to,
the ingest gap, and how to close it.

## End-to-end flow

Assumes witnesses are already in `orig_*_enc` form (see link above).

```
hash_recorder (foreman/)     →  h1 chain over original macroblock pixels (reference; not a real camera)
EditOnlyCircuit Nova proof   →  IVC state (h1, h2) in-circuit
hash_verifier_lossless       →  h2 via edit_native (reference; should match proof z[1])
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
3. Preview with `Removing::edit_native` in a small script, or extend `native_edit_export_yuv`

Lossy-path references: `edit_*_decider` and `hash_verifier_*` for each gadget.

## Branch

`edit-only-proof`
