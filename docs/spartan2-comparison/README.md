# Spartan2 / NeutronNova Comparison Study

This directory documents the effort to compare Eva's current **Nova + CycleFold + Groth16**
prover stack against **Microsoft Spartan2** (monolithic Spartan + NeutronNova folding).

## Motivation

Eva authenticates lossily encoded video via an incrementally verifiable computation (IVC).
Each step proves `256` macroblocks (`BLOCKS_PER_STEP`). The prover currently uses:

- **Nova NIFS** for folding step instances (cross-term vector `compute_t`, Pedersen MSM)
- **CycleFold** on a secondary curve (Grumpkin) for commitment folding
- **Custom lookup arguments** inside the step circuit
- **Groth16 decider** for the final succinct proof

Recent folding schemes — especially **NeutronNova** ([ePrint 2024/1606](https://eprint.iacr.org/2024/1606)) —
fold via zero-check / sum-check rather than Nova-style cross-term commitments.
**Spartan2** ([crates.io](https://crates.io/crates/spartan2)) ships production Rust
implementations of both Spartan and NeutronNova, with a `prep_prove` → `prove` split
that may align well with video (precomputable block data vs online challenges).

The goal of this branch is **not** an immediate production migration. It is a structured
benchmark and porting study to answer:

1. Is Nova folding or the step circuit the bottleneck?
2. Would NeutronNova improve wall-clock time and/or peak RAM on Eva workloads?
3. What is the engineering cost of a full port (lookups, CycleFold removal, decider change)?

## Branch

All work lives on **`spartan2-comparison`**, branched from **`cpu-backend`**.

```
cpu-backend          ← CPU MSM / compute_t (merged baseline)
    └── spartan2-comparison   ← this study
```

## Repository layout

| Path | Purpose |
|------|---------|
| `docs/spartan2-comparison/` | Design docs, phases, metrics (you are here) |
| `comparison/` | Runnable benchmarks and shared helpers |
| `comparison/examples/r1cs_stats.rs` | Print Eva step-circuit R1CS dimensions |
| `comparison/examples/phase0_baseline.rs` | Phase 0: Eva stats + NeutronNova smoke test |
| `comparison/examples/phase1_benchmark.rs` | Phase 1: Nova vs NeutronNova timed comparison |
| `comparison/examples/phase1_scale.rs` | Phase 1: multi-scale sweep (blocks 4, 16, 64) |
| `comparison/examples/phase2_lookup_smoke.rs` | Phase 2: LogUp bellpepper + NeutronNova smoke |
| `video/src/decider/` | Groth16 [`Decider`] + transparent [`SpartanDecider`] |

## Quick start

```bash
# From repo root — print Eva step-circuit R1CS statistics
BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats

# Phase 0: Eva R1CS stats + NeutronNova smoke test
NUM_STEPS=4 cargo run --release -p comparison --example phase0_baseline

# Phase 1: Nova vs NeutronNova comparison
QUICK=1 cargo run --release -p comparison --example phase1_benchmark
MATCH_EVA=1 QUICK=1 cargo run --release -p comparison --example phase1_benchmark

# Multi-scale sweep (blocks 4, 16, 64)
MATCH_EVA=1 cargo run --release -p comparison --example phase1_scale

# Phase 2: LogUp (bellpepper) under NeutronNova — 1 MB pixels
NUM_QUERIES=384 cargo run --release -p comparison --example phase2_lookup_smoke

# Full Eva Nova-only (NeutronNova at 1.4M constraints may OOM)
BLOCKS_PER_STEP=256 SKIP_NN=1 cargo run --release -p comparison --example phase1_benchmark
```

Environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `BLOCKS_PER_STEP` | `256` | Macroblocks per Eva IVC step (must match examples) |
| `NUM_STEPS` | `4` | NeutronNova batch size in phase 0 (padded to next power of two internally) |
| `CIRCUIT_DEGREE` | `1024` | Placeholder bellpepper circuit size for NeutronNova baseline |

## Documents in this folder

- [ARCHITECTURE.md](./ARCHITECTURE.md) — side-by-side stack comparison (Nova vs NeutronNova)
- [LOOKUPS.md](./LOOKUPS.md) — Eva lookup argument porting notes (Phase 2)
- [OPTION_C_TRANSPARENT.md](./OPTION_C_TRANSPARENT.md) — complete transparent decider (full Groth16 statement)
- [PHASES.md](./PHASES.md) — phased port plan with status checklist
- [METRICS.md](./METRICS.md) — what we measure and how to record results

## Current status

- [x] Phase 0: branch, docs, R1CS stats, NeutronNova smoke test
- [x] Phase 1: Nova timing harness + matched-size placeholder benchmark
- [x] Phase 1: multi-scale sweep (`phase1_scale`, blocks 4/16/64)
- [x] Nova `prove_step` at full BLOCKS_PER_STEP=256 (~5.5 s prove, ~1.43M constraints)
- [x] Peak RSS for Nova full-scale run (~6.2 GB)
- [x] Full-scale NeutronNova at 1.43M constraints (~5.1 s/step, ~6.7 GB RSS)
- [x] Phase 2.0: lookup witness layout documented
- [x] Phase 2.0: bellpepper LogUp + NeutronNova smoke (Q=16, 384, 2320)
- [x] Phase 3: dual deciders in `video` (`Decider` Groth16 + `SpartanDecider`)
- [ ] Phase 2: FS challenge + encode gadgets (full bellpepper Eva step)
- [ ] Full video pipeline comparison + recorded dual-path timings (Phase 3 complete)

## Key blockers (documented early)

1. **Circuit frontend mismatch** — Eva uses `ark-r1cs-std`; Spartan2 uses **bellpepper**.
   Direct R1CS matrix reuse is possible for analysis, but NeutronNova proving requires
   `SpartanCircuit` implementations.

2. **Lookup arguments** — Eva's custom lookup tables are embedded in Nova's augmented circuit.
   NeutronNova expects reductions to zero-check (Lasso-style). This is the largest porting item.

3. **CycleFold** — NeutronNova does not use Nova's two-curve CycleFold pattern; the comparison
   must account for different recursion architectures.

4. **Decider** — Groth16 (`Decider`) remains the production path (tiny proofs). A
   **complete transparent** backend (`SpartanDecider`) proves the same `DeciderEthCircuit`
   (`OPTION_C_TRANSPARENT.md`); proof size / verify cost still favor Groth16 for EVM.

## References

- [Nova (2021/370)](https://eprint.iacr.org/2021/370)
- [CycleFold (2023/1192)](https://eprint.iacr.org/2023/1192)
- [NeutronNova (2024/1606)](https://eprint.iacr.org/2024/1606)
- [Spartan2 crate](https://docs.rs/spartan2/latest/spartan2/)
- [Spartan2 GitHub](https://github.com/microsoft/Spartan2)
