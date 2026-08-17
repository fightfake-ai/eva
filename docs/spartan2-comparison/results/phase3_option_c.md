# Phase 3 / Option C results — Groth16 vs transparent Spartan (updated 2026-08-17)

Full architecture: [`OPTION_C_TRANSPARENT.md`](../OPTION_C_TRANSPARENT.md).

## Goal

Keep Nova IVC (steps + lookups). Compare **final compression**:

| Path | Final proof | Covers | Trusted setup? |
|------|-------------|--------|----------------|
| **Groth16** (`video::decider`) | Primary + CycleFold + device sigma | Yes |
| **Spartan full** (`SPARTAN_MODE=full`) | Same `DeciderEthCircuit` R1CS via matrix Spartan | **No** |
| **Spartan primary** (`SPARTAN_MODE=primary`) | Primary relaxed R1CS only (bellpepper) | No |

## Workload

- Circuit: `EditOnlyCircuit` + `Brightness`
- Synthetic non-zero macroblocks (no `DATA_PATH`)
- `BLOCKS_PER_STEP=4`, `NUM_STEPS=4` (need ≥4 so `U.cmE ≠ ∞` for Decider verify)
- DeciderEth size at this scale: **~7.0M constraints** (see `count_decider_constraints`)

## Results (developer Mac, `--release`)

### A. Groth16 vs Spartan **primary** (earlier spike)

| Backend | Setup | Prove | Verify | Proof size | Notes |
|---------|-------|-------|--------|------------|-------|
| Nova IVC (shared) | preprocess ~2.5 s | IVC ~0.9 s | — | — | 86,472 primary constraints |
| **Groth16 DeciderEth** | **115.8 s** | **113.2 s** | **4.2 ms** | **256 B** | full decider |
| **SpartanZk primary** | **~0.85 s** | **~0.95 s** | **~51 ms** | **~84 KB** | primary only |

### B. Spartan **full** transparent DeciderEth (measured 2026-08-17)

| Backend | Setup | Prove (synth+convert+core) | Verify | Proof size | Notes |
|---------|-------|----------------------------|--------|------------|-------|
| Nova IVC | preprocess 2.5 s | IVC 0.84 s | — | — | 86,472 primary constraints |
| **Spartan matrix full** | **23 ms** | **79.3 s** (synth 13.2 + convert 58.8 + core 7.2) | **3.73 s** | **~370 KB** | 6,963,321 cons (pad 8,388,608); vars 7,040,052; io 93 |

Convert (ark sparse → Spartan CSR + remap) dominates wall time at this scale; the sum-check
core prove is ~7 s. Verify is ~900× slower than Groth16’s ~4 ms, but still interactive-OK.
Proof is ~1,400× larger than Groth16’s 256 B.

## How to run

```bash
# Transparent full statement (recommended)
SKIP_G16=1 SPARTAN_MODE=full \
  cargo run --release -p comparison --example phase3_option_c

# Dual path (Groth16 setup ~2 min)
SPARTAN_MODE=full cargo run --release -p comparison --example phase3_option_c

# Primary-only (incomplete)
SKIP_G16=1 SPARTAN_MODE=primary \
  cargo run --release -p comparison --example phase3_option_c

SKIP_SPARTAN=1   # Groth16 only
BLOCKS_PER_STEP=4 NUM_STEPS=4
```

## Interpretation

- **Full transparent path** proves the same R1CS as Groth16 (primary + CycleFold + sigma).
- Groth16 remains best for tiny proofs / fast verify / EVM; requires trusted setup.
- Spartan full avoids toxic waste; expect larger proofs and slower verify at ~7M constraints.
- Primary-only Spartan stays useful as a cheap folding check, not a drop-in decider replacement.

## Code

- `comparison/src/option_c.rs` — dual-path runner (`SPARTAN_MODE`)
- `comparison/src/ark_to_spartan.rs` — ark → Spartan convert + prove
- `comparison/src/bellpepper/relaxed_check.rs` — primary-only path
- `third_party/spartan2/` — patched Spartan2 0.9
- `comparison/examples/phase3_option_c.rs`
