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

Native helpers: `hash_orig_macroblock`, `hash_edited_macroblock` (re-exported from `video`).

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

`EditOnlyCircuit<F, E>` is generic over `EditGadget` (`Brightness`, `Removing` for crop, etc.).
For crop, copy the edit-config loop from `edit_crop_decider` / `hash_verifier_crop` and use
`Removing` instead of `Brightness`.

## Branch

`edit-only-proof`
