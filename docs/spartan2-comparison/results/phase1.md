# Phase 1 results — 2026-06-26

## Environment

- Machine: developer Mac (darwin)
- Profile: `--release`, features: `cpu`
- NeutronNova: BN254 placeholder squaring circuit (NOT Eva logic)
- Nova: Eva `EditEncodeCircuit` + augmented Nova step (real Eva stack)

## Quick run (`QUICK=1`, BLOCKS_PER_STEP=4)

Eva augmented constraints at 4 blocks/step: **96,312**

### Unmatched placeholder (1000 constraints)

| Backend | Operation | Time | Constraints |
|---------|-----------|------|-------------|
| Nova | prove_step | 196.6 ms | 96,312 |
| Nova | compute_cmT | 83.9 ms | 96,312 |
| Nova | preprocess | 2576.1 ms | 96,312 |
| NeutronNova | prove (batch, 2 steps) | 32.6 ms | 1,000 × 2 |
| NeutronNova | prove (per step) | 16.3 ms | 1,000 |

*Not comparable — different constraint counts.*

### Matched placeholder (`MATCH_EVA=1`, degree=96311)

| Backend | Operation | Time | Constraints |
|---------|-----------|------|-------------|
| Nova | prove_step | 204.9 ms | 96,312 |
| Nova | compute_cmT | 87.1 ms | 96,312 |
| NeutronNova | prove (batch, 2 steps) | 677.4 ms | 96,312 × 2 |
| NeutronNova | prove (per step) | 338.7 ms | 96,312 |

**At matched ~96k constraints on quick Eva scale:**

- Nova `prove_step` is **~1.7× faster** than NeutronNova amortized prove per step.
- Important caveat: Nova `prove_step` is one folding step; NeutronNova `prove` includes
  multi-fold + Spartan final SNARK for the batch.

## Full Eva scale (BLOCKS_PER_STEP=256)

Not yet run — ~1.43M constraints, expect long runtime and high RAM.
Run manually:

```bash
BLOCKS_PER_STEP=256 MATCH_EVA=1 NUM_STEPS=2 \
  cargo run --release -p comparison --example phase1_benchmark
```

## Commands

```bash
# Quick iteration
QUICK=1 cargo run --release -p comparison --example phase1_benchmark

# Matched constraint count (uses Eva's constraint count for placeholder degree)
QUICK=1 MATCH_EVA=1 cargo run --release -p comparison --example phase1_benchmark
```

## Phase 1 interpretation

- Infrastructure works: side-by-side Nova vs NeutronNova timing harness.
- At small/matched scale, **Nova folding step beats NeutronNova batch prove** — but
  this compares different things (single fold vs fold+SNARK).
- Full-scale Eva (1.43M constraints) benchmark still needed before any migration decision.
- Still no Eva logic in NeutronNova — placeholder only.

## Next steps

- Run full-scale benchmark at BLOCKS_PER_STEP=256
- Record peak RSS with `/usr/bin/time -l`
- Phase 2: lookup argument port (if full-scale numbers warrant it)
