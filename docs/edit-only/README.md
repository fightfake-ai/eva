# Edit-only proofs

This branch adds an **edit-only** IVC circuit that proves video transformations
without H.264 encoding constraints.

## Motivation

The full Eva step circuit (`EditEncodeCircuit`) proves:

1. Edit gadget (crop, brightness, …)
2. H.264 forward encode (residual → DCT → quant)
3. Binding to predictor / QP / coefficients from JM dumps

That is ~1.4M constraints per step at `BLOCKS_PER_STEP=256`. For many use cases
you only need to attest **“these original pixels, after edit E, produce these
edited pixels”** — without tying the proof to a compressed bitstream.

## New code

| Path | Purpose |
|------|---------|
| `video/src/edit_only.rs` | `EditOnlyCircuit` implementing `FCircuit` |
| `video/examples/edit_bright_only.rs` | Runnable Nova prove + verify demo |
| `video/src/edit_only.rs` tests | Native vs constraints + augmented circuit |

## What the edit-only circuit checks

Per macroblock:

- **Witness:** original Y/U/V pixels (from `foreman/orig_*`)
- **Constraint:** `edit_circuit` (e.g. brightness multiply)
- **h1:** Griffin hash of **original** pixels (recorder binding, same idea as full Eva)
- **h2:** Griffin hash of **edited** pixels + edit config (e.g. brightness scale)

No `pred_*`, `coeff_*`, `type_enc`, or lookup-based quant constraints.

## Run

```bash
export DATA_PATH=/path/to/data_parsed

# Fast smoke test
QUICK=1 cargo run --release -p video --example edit_bright_only

# Unit tests (no dataset required)
cargo test -p video edit_only --release
```

## Extending to other edits

`EditOnlyCircuit<F, E>` is generic over `EditGadget` (`Brightness`, `Removing` for crop, etc.).
Copy `edit_bright_only.rs` and change the `Op` type and edit config.

## Branch

`edit-only-proof` (from `spartan2-comparison`).
