# Architecture: Eva (Nova) vs Spartan2 (NeutronNova)

This document maps Eva's current prover pipeline to Spartan2's NeutronNova API so we know
exactly what would need to change in a full port.

## Eva today (Nova + CycleFold)

```
Video blocks (256/step)
        │
        ▼
┌───────────────────────────────────────┐
│  EditEncodeCircuit  (FCircuit)        │  ark-r1cs-std
│  + LookupArgument histogram/constraints │
└───────────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────────┐
│  AugmentedFCircuit  (F')              │  Nova step circuit
│  - verifies prior folded instance     │
│  - runs F, updates state z            │
│  - embeds CycleFold verifier gadgets  │
└───────────────────────────────────────┘
        │
        ▼  prove_step (streaming IVC)
┌───────────────────────────────────────┐
│  NIFS                                 │
│  - compute_t (MVM on A,B,C)           │  ← CPU or CUDA backend
│  - Pedersen commit to T               │  ← MSM
│  - fold witness + committed instance  │
└───────────────────────────────────────┘
        │
        ▼  parallel on C2 (Grumpkin)
┌───────────────────────────────────────┐
│  CycleFold NIFS                       │  folds cmW, cmE commitments
└───────────────────────────────────────┘
        │
        ▼  after all steps
┌───────────────────────────────────────┐
│  Groth16 Decider                      │  DeciderEthCircuit → Ethereum verifier
└───────────────────────────────────────┘
```

### Curves and commitments

| Component | Eva |
|-----------|-----|
| Primary curve C1 | BN254 (`G1Projective`) |
| Secondary curve C2 | Grumpkin |
| Step commitments | Pedersen on C1 |
| CycleFold commitments | Pedersen on C2 |
| Final proof | Groth16 on BN254 |

### Hot paths (where CPU backend matters)

Defined in `folding-schemes/src/lib.rs`:

- **`MVM::compute_t`** — sparse matrix-vector products + cross-term formula per constraint row
- **`MSM::*`** — Pedersen commitment MSMs
- **`MVM::update_e`** — error accumulator update during folding

Code references:

- NIFS prover: `folding-schemes/src/folding/nova/nifs.rs`
- Nova IVC loop: `folding-schemes/src/folding/nova/mod.rs`
- Step circuit: `video/src/lib.rs` (`EditEncodeCircuit`)

---

## Spartan2 / NeutronNova target

```
Uniform step circuits × N  +  1 core circuit
        │
        ▼
┌───────────────────────────────────────┐
│  SpartanCircuit (bellpepper)          │
│  - shared / precommitted / aux split  │
│  - prep_prove caches Az,Bz,Cz         │
└───────────────────────────────────────┘
        │
        ▼  batch (non-recursive NeutronNova)
┌───────────────────────────────────────┐
│  NeutronNova NIFS                     │
│  - zero-check / sum-check folding     │
│  - Hyrax PCS commitments              │
└───────────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────────┐
│  Spartan SNARK on folded instance     │  relaxed R1CS
└───────────────────────────────────────┘
```

### Spartan2 API surface (NeutronNova ZK)

From `spartan2::neutronnova_zk::NeutronNovaZkSNARK`:

```rust
// 1. Key generation from circuit prototypes
let (pk, vk) = NeutronNovaZkSNARK::<E>::setup(step_circuit, core_circuit, num_steps)?;

// 2. Precompute / commit precommitted witness (amortizable)
let prep = NeutronNovaZkSNARK::<E>::prep_prove(&pk, step_circuits, core_circuit, is_small)?;

// 3. Online prove (challenges, aux witness)
let (proof, prep) = NeutronNovaZkSNARK::<E>::prove(&pk, step_circuits, core_circuit, prep, is_small)?;

// 4. Verify
proof.verify(&vk, num_instances)?;
```

The `is_small` flag enables fast paths when witness values fit in small integers
(bit-width-aware MSM — relevant for Eva's `8`-bit macroblock coefficients).

### Curves in Spartan2

Spartan2 provides `Bn254Engine` (same base field as Eva's BN254 step circuit).
We use BN254 in Phase 0 so curve mismatch does not confound early benchmarks.
Grumpkin / CycleFold has no direct Spartan2 equivalent in Eva's sense.

---

## Mapping table

| Eva concept | Spartan2 / NeutronNova equivalent | Port difficulty |
|-------------|-----------------------------------|-----------------|
| `FCircuit` / `EditEncodeCircuit` | `SpartanCircuit` step circuit | **High** (re-write in bellpepper) |
| `AugmentedFCircuit` | Likely absorbed into step + core circuits | **High** |
| `LookupArgument` | Lasso-style zero-check reduction | **Very high** |
| `NIFS::compute_t` | Internal NeutronNova folding | N/A (replaced) |
| `CycleFold` | Not used; different recursion model | **High** (redesign) |
| `prove_step` streaming | Batch `prep_prove` + `prove` | **Medium** (API redesign) |
| Groth16 decider | Spartan final SNARK | **High** (verifier change) |
| `ExternalInputs` (video blocks) | `precommitted` witness in `SpartanCircuit` | **Medium** |

---

## What Phase 0 validates

Phase 0 **does not** port Eva's circuit. It establishes:

1. **R1CS dimensions** of Eva's real step and augmented circuits (constraint count, variables, sparsity).
2. **Spartan2 toolchain works** on BN254 in this workspace.
3. **Benchmark harness** structure for later apples-to-apples timing.

Phase 1 will target a bellpepper circuit whose constraint count matches Eva's **step-only**
R1CS (without lookups initially), then NeutronNova prove timing at matching `NUM_STEPS`.
