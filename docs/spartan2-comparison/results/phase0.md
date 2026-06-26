# Phase 0 results — 2026-06-26

## Environment

- Machine: developer Mac (darwin)
- Profile: `--release`
- Features: `cpu` (no CUDA)
- `BLOCKS_PER_STEP=256`, `NUM_STEPS=2`, `CIRCUIT_DEGREE=64`

## Eva (Nova) — R1CS dimensions

| Circuit | Constraints | Variables | Public IO | Committed | nnz(A,B,C) | nnz/constraint |
|---------|-------------|-----------|-----------|-----------|------------|----------------|
| eva-step-only | 1,354,761 | 1,846,540 | 3 | 594,176 | (14.7M, 18.3M, 5.8M) | 28.6 |
| eva-augmented | 1,429,212 | 1,916,607 | 3 | 594,176 | (15.0M, 18.9M, 5.9M) | 27.9 |

**Interpretation:**

- The augmented Nova step circuit adds ~74k constraints (~5.5%) over step-only F circuit.
- Phase 1 placeholder should target **~1.43M constraints** to match what Nova folds per step.
- Lookup arguments dominate committed witness count (594k committed vars).

## NeutronNova (Spartan2) — smoke test

Placeholder squaring circuit on **BN254** (NOT Eva logic):

| NUM_STEPS | CIRCUIT_DEGREE | setup | prep_prove | prove | verify |
|-----------|----------------|-------|------------|-------|--------|
| 2 | 64 | 70.8 ms | 0.0 ms | 22.6 ms | 15.9 ms |

**Notes:**

- `prep_prove` reported 0.0 ms — constraints are in `synthesize` for this placeholder; Phase 1
  will move precomputable witness into `precommitted` once bellpepper port begins.
- Verify passes on BN254 after moving constraints from `precommitted` → `synthesize`.
- Reference SHA-256 test on T256HyraxEngine passes (`neutronnova_sha256_reference_t256` unit test).

## Phase 0 exit criteria

- [x] Eva augmented-circuit constraint count documented
- [x] NeutronNova setup → prove → verify on BN254
- [x] No regressions to `folding-schemes` / `video`

## Next (Phase 1)

1. Size `PlaceholderStepCircuit.degree` to ~1.43M constraints (or equivalent R1CS)
2. Add Nova `prove_step` timing harness in `comparison` crate
3. Record peak RSS with `/usr/bin/time -l`
