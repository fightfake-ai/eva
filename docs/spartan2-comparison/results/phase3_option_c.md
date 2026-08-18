# Dual decider results — Groth16 vs transparent Spartan (2026-08-17)

Architecture: [`OPTION_C_TRANSPARENT.md`](../OPTION_C_TRANSPARENT.md).

## Goal

Keep Nova IVC (steps + lookups). Two **parallel** final SNARKs on the same
`DeciderEthCircuit` (primary + CycleFold + device sigma).

The incomplete primary-only Spartan path was removed.

## Workload

- Circuit: `EditOnlyCircuit` + `Brightness`
- Synthetic / QUICK-scale: `BLOCKS_PER_STEP=4`, `NUM_STEPS=4`
- DeciderEth size: **~7.0M constraints**

## Results (developer Mac, `--release`)

| Backend | Setup | Prove | Verify | Proof size | Notes |
|---------|-------|-------|--------|------------|-------|
| Nova IVC | preprocess ~2.5 s | IVC ~0.84 s | — | — | 86,472 primary constraints |
| **Groth16 `Decider`** | **115.8 s** | **113.2 s** | **4.2 ms** | **256 B** | trusted setup |
| **`SpartanDecider`** | **23 ms** | **79.3 s** | **3.73 s** | **~370 KB** | 6,963,321 cons (pad 8.4M) |

Spartan prove split (same run): synthesize 13.2 s + convert 58.8 s + core 7.2 s.

## How to run

```bash
QUICK=1 cargo run --release -p video --example edit_lossless_decider
QUICK=1 DECIDER=spartan cargo run --release -p video --example edit_lossless_decider
```

## Code

- `video/src/decider/mod.rs` — `DeciderEthCircuit` + Groth16 `Decider`
- `video/src/decider/spartan.rs` — `SpartanDecider`
- `third_party/spartan2/` — patched Spartan2 0.9
