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

## Quick start

```bash
# From repo root — print Eva step-circuit R1CS statistics
BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats

# Phase 0: Eva R1CS stats + NeutronNova reference prove on BN254
NUM_STEPS=4 cargo run --release -p comparison --example phase0_baseline
```

Environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `BLOCKS_PER_STEP` | `256` | Macroblocks per Eva IVC step (must match examples) |
| `NUM_STEPS` | `4` | NeutronNova batch size in phase 0 (padded to next power of two internally) |
| `CIRCUIT_DEGREE` | `1024` | Placeholder bellpepper circuit size for NeutronNova baseline |

## Documents in this folder

- [ARCHITECTURE.md](./ARCHITECTURE.md) — side-by-side stack comparison (Nova vs NeutronNova)
- [PHASES.md](./PHASES.md) — phased port plan with status checklist
- [METRICS.md](./METRICS.md) — what we measure and how to record results

## Current status (Phase 0)

- [x] Branch created, documentation scaffold
- [x] `comparison` crate with Eva R1CS stats extraction
- [x] NeutronNova smoke test on BN254 via Spartan2
- [ ] Nova `prove_step` timing harness in `comparison` crate
- [ ] Bellpepper re-synthesis of Eva step circuit (Phase 1)
- [ ] Lookup argument port (Phase 2)
- [ ] Full video pipeline comparison (Phase 3)

## Key blockers (documented early)

1. **Circuit frontend mismatch** — Eva uses `ark-r1cs-std`; Spartan2 uses **bellpepper**.
   Direct R1CS matrix reuse is possible for analysis, but NeutronNova proving requires
   `SpartanCircuit` implementations.

2. **Lookup arguments** — Eva's custom lookup tables are embedded in Nova's augmented circuit.
   NeutronNova expects reductions to zero-check (Lasso-style). This is the largest porting item.

3. **CycleFold** — NeutronNova does not use Nova's two-curve CycleFold pattern; the comparison
   must account for different recursion architectures.

4. **Decider** — Eva finishes with Groth16 (`DeciderEthCircuit`). Spartan2 finishes with
   Spartan over a relaxed R1CS instance. Proof size and on-chain verifier cost differ.

## References

- [Nova (2021/370)](https://eprint.iacr.org/2021/370)
- [CycleFold (2023/1192)](https://eprint.iacr.org/2023/1192)
- [NeutronNova (2024/1606)](https://eprint.iacr.org/2024/1606)
- [Spartan2 crate](https://docs.rs/spartan2/latest/spartan2/)
- [Spartan2 GitHub](https://github.com/microsoft/Spartan2)
