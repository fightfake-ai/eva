# Eva lookup arguments — porting notes for Phase 2

This document describes Eva's custom lookup argument and what a NeutronNova/Spartan2 port
would require. Read alongside [ARCHITECTURE.md](./ARCHITECTURE.md).

## What Eva uses today

Eva embeds a **LogUp-style lookup argument** in every step circuit synthesis:

1. **`LookupArgument::set_table`** — lookup table (256 entries for 8-bit macroblock values)
2. **`build_histo`** — count occurrences of each table value in committed witness columns
3. **`generate_lookup_constraints`** — enforce LogUp identity with random challenge `c`

Implementation: `folding-schemes/src/frontend/mod.rs` (`LookupArgument` struct).

Called from:
- `video/src/lib.rs` — `EditEncodeCircuit::generate_step_constraints` (per macroblock)
- `folding-schemes/src/folding/nova/mod.rs` — `AugmentedFCircuit::run` + `prove_step`
- `comparison/src/eva_step.rs` — R1CS stats extraction

### Constraint impact

At `BLOCKS_PER_STEP=256` (Phase 0 numbers):

| Circuit | Constraints | Committed witnesses |
|---------|-------------|---------------------|
| step-only | 1,354,761 | 594,176 |
| augmented | 1,429,212 | 594,176 |

Lookups account for the majority of **committed** variables (blinded macroblock coefficients).

### LogUp structure (simplified)

For table entries `T` and query values `q_j` with random challenge `c`:

```
LHS: Σ_i  1 / (c - T_i)     (weighted by histogram counts)
RHS: Σ_j  1 / (c - q_j)
```

Enforced as R1CS via witness inverses (`compute_lhs`, `compute_rhs` in `frontend/mod.rs`).

## Why this blocks a naive NeutronNova port

Spartan2 NeutronNova expects circuits as **`SpartanCircuit`** in **bellpepper**, with:

- `shared` / `precommitted` / `synthesize` witness split
- Reductions to **zero-check** for non-native operations

Eva's lookup is:

- Implemented directly in **ark-r1cs-std** constraint generation
- Tightly coupled to Nova's **committed witness** layout (`num_committed_variables`)
- Applied **after** step circuit synthesis via `build_histo` + `generate_lookup_constraints`

There is no drop-in mapping to Spartan2's API without reimplementing the lookup in bellpepper.

## Port options (Phase 2 evaluation)

### Option A: Reimplement LogUp in bellpepper

- Rewrite `LookupArgument` using bellpepper gadgets
- Integrate histogram building into `precommitted` witness phase
- **Effort:** high; **fidelity:** highest

### Option B: Lasso / zero-check reduction (Spartan2-native)

- NeutronNova paper reduces lookups to zero-check instances
- Spartan2 may expose patterns for this (see Jolt/Lasso lineage)
- **Effort:** very high; **potential gain:** best asymptotics if it works

### Option C: Hybrid — keep Nova step, Spartan decider only

- Continue Nova folding for step + lookups
- Replace only Groth16 decider with Spartan
- **Effort:** medium; **does not** capture NeutronNova folding wins

### Option D: R1CS export bridge (research)

- Export arkworks R1CS matrices → Spartan2 `R1CSShape`
- Prove exported R1CS without re-synthesis
- **Effort:** medium for single-instance; **IVC/streaming:** unclear

## Recommended Phase 2 order

1. **Document lookup witness layout** — which committed columns are lookup queries
2. **Prototype Option A** for a **single macroblock** in bellpepper (not full 256)
3. **Measure** constraint count vs arkworks version — must match for soundness
4. Only then integrate into `SpartanCircuit` step prototype

## Files to modify in a full port

| File | Role |
|------|------|
| `folding-schemes/src/frontend/mod.rs` | Current LogUp implementation (reference) |
| `video/src/lib.rs` | Macroblock constraints + lookup table setup |
| `folding-schemes/src/folding/nova/circuits.rs` | AugmentedFCircuit integration |
| New: `comparison/src/bellpepper/lookup.rs` | bellpepper port (Phase 2) |
| New: `comparison/src/bellpepper/step.rs` | Eva step as SpartanCircuit (Phase 2+) |

## Open questions

- Can Spartan2 `is_small=true` fast path apply to Eva's 8-bit coefficients in lookups?
- Does NeutronNova batch proving align with Eva's streaming `prove_step`, or must we batch macroblock steps?
- What is the peak RAM of lookup constraints at 1.43M rows in Spartan2 vs Nova?

## References

- Eva LogUp: `folding-schemes/src/frontend/mod.rs`
- Lasso: [ePrint 2022/1380](https://eprint.iacr.org/2022/1380)
- NeutronNova reductions: [ePrint 2024/1606](https://eprint.iacr.org/2024/1606) §1.2
