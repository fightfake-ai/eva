# Track 1 — Still-image EditOnly (native)

**Status:** Done (2026-08-05)  
**Scope:** Native still-image path only — **no** wasm prove.  
**Repos:** `eva-miha` (`fightfake-ai/eva`) + `fightfake-toolkit`

## Goal

Treat a **still image** as a first-class input to the existing EditOnly prove pipeline:

```
PNG / JPEG / WebP / …
    → RGB → YUV 4:2:0 (crop to multiples of 16)
    → Eva macroblocks (num_frames = 1)
    → EditOnly gadget (brightness / redact / …)
    → FFPB proof.bin + C2PA (via toolkit)
```

Video remains supported; a still is just **one frame** of macroblocks.

## What changed

### eva-miha

| Path | Role |
|------|------|
| `video/src/rgb_yuv.rs` | `rgb8_to_yuv420p`, `crop_to_macroblock_grid` (no image decoder in lib) |
| `video/examples/image_to_macroblocks.rs` | PNG/JPEG → `orig_*_enc` dumps |
| `docs/edit-only/README.md` | Still-image quick start |

### fightfake-toolkit

| Path | Role |
|------|------|
| `fightfake-cli/src/image_ingest.rs` | Detect stills; decode with `image` crate; RGB→YUV |
| `fightfake-cli/src/workflow.rs` | Still branch in `run_prove_edit`; auto `--blocks-per-step`; 1-frame `capture.mp4` for C2PA |
| `fightfake-cli/src/main.rs` | Help text; redact `--redact-frame-end` defaults to `1` for stills |
| `testdata/images/toy64.png` | 64×64 smoke-test asset |
| `README.md` | `prove-edit` accepts images |

## Behaviour details

1. **Ingest:** Stills decoded in-process (no ffmpeg). Videos still use ffmpeg/ffprobe.
2. **Crop:** Non-multiple-of-16 sizes → top-left crop to floor(w/16)×16 × floor(h/16)×16, with a log line.
3. **`--blocks-per-step`:** Default `256` often does not divide a small still (e.g. 64×64 → 16 MBs). Workflow auto-adjusts to macroblocks-per-frame (or total MBs).
4. **C2PA:** Signer requires matching container types. Stills are re-wrapped as a 1-frame `capture.mp4` before signing; edited output remains `edited.mp4`.
5. **Redact on stills:** If `--redact-frame-end` is omitted/`0`, it defaults to `1`.

## How to run

### Toolkit Level-0 (fast: edit + hashes + stub proof + C2PA)

```bash
cd fightfake-toolkit
cargo run --release -p fightfake-cli -- \
  prove-edit --input testdata/images/toy64.png \
  --gadget brightness --out-dir out-photo/
```

Redact example:

```bash
cargo run --release -p fightfake-cli -- \
  prove-edit --input testdata/images/toy64.png \
  --gadget redact --redact-width 32 --redact-height 32 \
  --out-dir out-redact/
```

### Toolkit Level-1 (real Nova + Groth16)

```bash
cargo run --release -p fightfake-cli --features eva-backend -- \
  prove-edit --input testdata/images/toy64.png \
  --gadget brightness --out-dir out-photo-zk/
```

Requires Eva git pin that includes EditOnly (existing). For local `eva-miha` changes to gadgets/helpers used only in examples, Level-0 does not need a pin bump; Level-1 uses the toolkit’s pinned Eva `rev`.

### Eva macroblock dump only

```bash
cd eva-miha
cargo run --release -p video --example image_to_macroblocks -- \
  photo.png data_parsed/photo

export DATA_PATH="$(pwd)/data_parsed"
VIDEO=photo QUICK=1 cargo run --release -p video --example edit_bright_only
```

## Smoke test results (2026-08-05)

| Check | Result |
|-------|--------|
| `image_ingest` unit tests | Pass |
| Level-0 `prove-edit` on `toy64.png` | Pass — 64×64, 16 MBs, blocks-per-step auto 16, C2PA OK |
| `image_to_macroblocks` on `toy64.png` | Pass — 16 macroblocks written |

## Explicitly out of scope

- Wasm prove (still blocked on Groth16 PK size — see `docs/SPIKE_A.md`)
- Proving RGB→YUV inside the circuit (ingest remains trusted)
- Native PNG output instead of 1-frame MP4 for C2PA assets
- Spartan / NeutronNova migration

## Next options

1. Bump toolkit Eva `rev` after pushing `eva-miha` if Level-1 should pick up any Eva-side API used by CLI (not required for Track 1 Level-0).
2. Optional: emit still PNG for preview alongside 1-frame MP4.
3. Optional micro-spike: measure Groth16 PK size for 1–4 macroblock stills (Track 2 / wasm feasibility).
