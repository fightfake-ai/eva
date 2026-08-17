# comparison crate

Runnable benchmarks for the Spartan2 / NeutronNova comparison study.

See [`../docs/spartan2-comparison/README.md`](../docs/spartan2-comparison/README.md) for the full plan.

## Examples

```bash
# Print Eva step-circuit R1CS dimensions
BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats

# Phase 0: Eva stats + NeutronNova smoke test
NUM_STEPS=4 cargo run --release -p comparison --example phase0_baseline

# Phase 1: side-by-side Nova vs NeutronNova
QUICK=1 cargo run --release -p comparison --example phase1_benchmark

# Phase 2: LogUp under NeutronNova (1 MB pixels)
NUM_QUERIES=384 cargo run --release -p comparison --example phase2_lookup_smoke
```

## Modules

| Module | Description |
|--------|-------------|
| `r1cs_stats` | `R1csStats` struct and printing helpers |
| `eva_step` | Synthesize Eva step / augmented circuits via arkworks |
| `nova_baseline` | Nova prove_step / compute_cmT / preprocess timings |
| `neutronnova` | Spartan2 NeutronNova reference benchmark on BN254 |
| `bellpepper` | Phase 2 LogUp gadgets + Option C relaxed-R1CS check |
| `option_c` | Nova IVC + Groth16 Decider vs Spartan primary SNARK |

```bash
# Option C dual-path (default 4 blocks, 4 steps)
cargo run --release -p comparison --example phase3_option_c
```

