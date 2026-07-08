# Metrics & benchmarking methodology

Consistent measurement is required for a fair Nova vs NeutronNova comparison.
This document defines **what** we record and **how**.

## Primary metrics

| Metric | Unit | Nova source | NeutronNova source |
|--------|------|-------------|-------------------|
| Constraint count | # | `R1csStats.num_constraints` | Spartan2 `ConstraintSystem::num_constraints` |
| Variable count | # | `R1csStats.num_variables` | bellpepper CS |
| Matrix nnz (A,B,C) | # | `R1csStats.nnz_*` | logged from Spartan2 shape metadata |
| Setup time | ms | `Nova::preprocess` | `NeutronNovaZkSNARK::setup` |
| Prep prove time | ms | N/A (no split) | `prep_prove` |
| Prove time | ms | `prove_step` or `compute_t`+MSM | `prove` |
| Verify time | ms | `Nova::verify` | `proof.verify` |
| Peak RSS | MB | `/usr/bin/time -l` or `/usr/bin/time -v` | same |
| Proof size | bytes | Groth16 proof serialize | `NeutronNovaZkSNARK` serialize |

## Secondary metrics

- **Time per macroblock** = total prove time / (NUM_STEPS × BLOCKS_PER_STEP)
- **Amortized prep** = prep_prove time / number of subsequent `prove` calls
- **Folding-only time** = Nova `compute_t` + `update_e` (exclude circuit synthesis)

## Hardware recording

Always log in results files:

```
CPU:        (e.g. Apple M2 Max)
RAM:        (e.g. 64 GB)
OS:         (e.g. macOS 15.5)
Rust:       (rustc --version)
Profile:    release
Features:   cpu (no cuda unless explicitly testing GPU)
Date:       YYYY-MM-DD
```

## Commands

### Eva R1CS stats

```bash
BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats 2>&1 | tee docs/spartan2-comparison/results/r1cs_stats.log
```

### Phase 0 NeutronNova smoke

```bash
NUM_STEPS=4 CIRCUIT_DEGREE=1024 \
  cargo run --release -p comparison --example phase0_baseline 2>&1 | tee docs/spartan2-comparison/results/phase0.log
```

### Peak memory (macOS)

```bash
/usr/bin/time -l cargo run --release -p comparison --example phase0_baseline
```

Look for `maximum resident set size` in stderr.

### Peak memory (Linux)

```bash
/usr/bin/time -v cargo run --release -p comparison --example phase0_baseline
```

Look for `Maximum resident set size`.

## Fair comparison rules

1. **Same curve where possible** — BN254 for both stacks in early phases.
2. **Same `BLOCKS_PER_STEP`** — default `256` matching Eva examples.
3. **Release builds only** for timing (`--release`).
4. **Warmup** — discard first run; report median of ≥3 runs for micro-benchmarks.
5. **Do not compare** Phase 0 NeutronNova placeholder constraint count to Eva augmented
   circuit directly — note circuit type in results table.

## Results template

Create `docs/spartan2-comparison/results/phaseN.md` with:

```markdown
# Phase N results — YYYY-MM-DD

## Environment
- CPU: ...
- RAM: ...
- ...

## Eva (Nova) — step circuit
| BLOCKS_PER_STEP | constraints | variables | nnz_A | nnz_B | nnz_C |
|-----------------|-------------|-----------|-------|-------|-------|
| 256             | ...         | ...       | ...   | ...   | ...   |

## NeutronNova (Spartan2)
| NUM_STEPS | CIRCUIT_DEGREE | setup_ms | prep_ms | prove_ms | verify_ms | peak_rss_mb |
|-----------|----------------|----------|---------|----------|-----------|-------------|
| 4         | 1024           | ...      | ...     | ...      | ...       | ...         |

## Notes
- ...
```

## Known asymmetries (document, don't hide)

| Asymmetry | Why it matters |
|-----------|----------------|
| Nova streams steps; NeutronNova batches | Compare per-step amortized cost |
| Eva augmented circuit >> step circuit | Report both R1CS stats |
| Nova uses Groth16 decider | Final proof size not comparable until Phase 3 |
| CycleFold overhead only in Nova | Nova numbers include Grumpkin folding |
