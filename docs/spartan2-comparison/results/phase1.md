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

## Scaling study (`phase1_scale`, `MATCH_EVA=1`, `NUM_STEPS=2`)

Placeholder degree matched to Eva augmented constraint count at each scale.

| blocks | constraints | nova_prove_ms | nova_cmT_ms | nova_preprocess_ms | nn_prove/step_ms | nn/nova |
|--------|-------------|---------------|-------------|--------------------|--------------------|---------|
| 4 | 96,312 | 390.0 | 216.7 | 5,664 | 877.8 | 2.25 |
| 16 | 159,900 | 462.8 | 225.0 | 6,647 | 658.3 | 1.42 |
| 64 | 413,640 | 4,262.1 | 1,722.7 | 28,268 | 1,597.0 | 0.37 |

**Observations:**

- Nova `prove_step` scales super-linearly between 16→64 blocks (~9× time for ~2.6× constraints),
  dominated by `compute_cmT` and witness MSM at larger R1CS sizes.
- NeutronNova placeholder prove/step improves relative to Nova as constraint count grows:
  at 64 blocks NN is **~2.7× faster** than Nova `prove_step` (ratio 0.37).
- Nova `preprocess` (one-time setup) grows from ~5.7 s (4 blocks) to ~28 s (64 blocks).
- Still comparing different operations: Nova = single fold step; NeutronNova = fold batch + Spartan SNARK.

Run:

```bash
MATCH_EVA=1 cargo run --release -p comparison --example phase1_scale
```

## Full Eva scale (`BLOCKS_PER_STEP=256`, `MATCH_EVA=1`, `NUM_STEPS=2`)

| Backend | Operation | Time | Constraints |
|---------|-----------|------|-------------|
| Nova | synthesis | 10,488 ms | 1,429,212 |
| Nova | preprocess | 44,020 ms | 1,429,212 |
| Nova | compute_cmT | 2,096 ms | 1,429,212 |
| Nova | prove_step | 5,972 ms | 1,429,212 |
| NeutronNova | setup | 3,079 ms | 1,429,212 |
| NeutronNova | prep_prove | 13.5 ms | 1,429,212 |
| NeutronNova | prove (batch, 2 steps) | 10,213 ms | 1,429,212 × 2 |
| NeutronNova | prove (per step) | 5,106 ms | 1,429,212 |
| NeutronNova | verify | 294 ms | batch |

**At full Eva scale (~1.43M constraints):**

- NeutronNova amortized prove/step is **~1.17× faster** than Nova `prove_step` (5.1 s vs 6.0 s).
- Caveat: placeholder squaring circuit, not Eva THASH/lookup logic; NN prove includes
  multi-fold + Spartan SNARK while Nova `prove_step` is a single fold.
- Scaling trend: NN advantage grows from medium scale (64 blocks, ~2.7×) but narrows at
  full scale — likely due to Spartan final SNARK dominating NN batch cost at 1.43M constraints.

### Peak RSS (full scale, `/usr/bin/time -l`)

| Run | max RSS |
|-----|---------|
| Nova only (`SKIP_NN=1`) | **6.19 GB** |
| Nova + NeutronNova | **6.68 GB** |

## Commands

```bash
# Quick iteration
QUICK=1 cargo run --release -p comparison --example phase1_benchmark

# Matched constraint count (uses Eva's constraint count for placeholder degree)
QUICK=1 MATCH_EVA=1 cargo run --release -p comparison --example phase1_benchmark

# Multi-scale sweep (blocks 4, 16, 64)
MATCH_EVA=1 cargo run --release -p comparison --example phase1_scale

# Full Eva Nova-only
BLOCKS_PER_STEP=256 SKIP_NN=1 MATCH_EVA=1 cargo run --release -p comparison --example phase1_benchmark
```

## Phase 1 interpretation

- Infrastructure works: side-by-side Nova vs NeutronNova timing harness with shared `phase1` module.
- At small scale (~96k), Nova folding step beats NeutronNova batch prove (~1.7–2.3×).
- At medium scale (~414k), NeutronNova placeholder **beats** Nova prove_step (~2.7×) — but
  this is a synthetic squaring circuit, not Eva's THASH/lookup logic.
- Full-scale Nova: **~6.0 s prove_step** + **~2.1 s compute_cmT** at 1.43M constraints on CPU.
- Full-scale NeutronNova placeholder: **~5.1 s prove/step** — modest ~17% win over Nova fold step,
  but on synthetic circuit without lookups.
- Phase 1 exit criterion (>2× improvement) **not met** at full scale on placeholder circuit.
- Lookup port (Phase 2) required before any migration decision.

## Next steps

- Phase 2: lookup argument port (see [LOOKUPS.md](../LOOKUPS.md))
- Re-run full-scale comparison after Eva step circuit is in bellpepper with real logic
