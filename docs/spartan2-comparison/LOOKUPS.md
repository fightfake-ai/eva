# Eva lookup arguments — porting notes for Phase 2

This document describes Eva's custom lookup argument and what a NeutronNova/Spartan2 port
requires. Read alongside [ARCHITECTURE.md](./ARCHITECTURE.md).

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

### Critical design: no explicit query registry

There is **no `register_query`**. Queries are **every committed CS variable** allocated
before `build_histo`. The LA only stores the table and histogram. Misses (value ∉ table)
panic via `.unwrap()` when incrementing the histogram.

### Constraint impact

At `BLOCKS_PER_STEP=256` (Phase 0 numbers):

| Circuit | Constraints | Committed witnesses |
|---------|-------------|---------------------|
| step-only | 1,354,761 | 594,176 |
| augmented | 1,429,212 | 594,176 |

At `BLOCKS_PER_STEP=1` (NoOp EditEncode, measured Phase 2):

| Circuit | Constraints | Committed |
|---------|-------------|-----------|
| step-only (F + LogUp) | **6,117** | **2,576** |
| augmented | **80,568** | **2,576** |

Lookups account for the majority of **committed** variables (blinded macroblock coefficients
and encode bit-length chunks).

### LogUp structure (simplified)

For table entries `T` and query values `q_j` with random challenge `c`:

```
LHS: Σ_i  e_i / (c - T_i)     (e_i = histogram multiplicity of T_i)
RHS: Σ_j  1 / (c - q_j)
```

Enforced as R1CS via witness inverses (`compute_lhs`, `compute_rhs` in `frontend/mod.rs`):

| Check | Constraint | Count |
|-------|------------|-------|
| LHS | `(c - T_i) · inv_i = e_i` | T = 256 |
| RHS | `(c - q_j) · inv_j = 1` | Q |
| Identity | `(Σ inv_i) · 1 = (Σ inv_j)` | 1 |

**Core LogUp cost: Q + T + 1 constraints.**

In real Nova prove, `c = Poseidon(cmQ)` after committing the full committed vector
(queries + histo) — see `folding-schemes/src/folding/nova/mod.rs`.

---

## Witness layout (committed columns)

For table `T = {0,1,…,255}` (`MB_BITS = 8`):

| Region | Storage | Count (1 MB, NoOp EditEncode) | Role |
|--------|---------|-------------------------------|------|
| **Queries `q[0..Q)`** | `Variable::Committed` | **Q = 2,320** | All values that must ∈ table |
| **Histo `e[0..256)`** | Committed, appended by `build_histo` | **256** | Multiplicity of each `T_i` |
| **Table `T`** | LA field only (not CS vars) | 256 constants | Public range |
| **Challenge `c`** | Instance / public input | 1 | LogUp challenge |
| **LHS invs** | Witness | 256 | `e_i/(c-T_i)` |
| **RHS invs** | Witness | Q | `1/(c-q_j)` |

**Total committed after histo:** **2,576** = 2,320 + 256.

### What becomes a query (EditEncode, NoOp)

Anything allocated with `AllocationMode::Committed` / `new_committed` during the step,
**before** `build_histo`:

1. **Pixels:** Y 16×16 + U 8×8 + V 8×8 = **384** (`MatrixVar::new_committed`)
2. **Predictions:** same sizes = **384** (NoOp: non-constant encoding path)
3. **Encode intermediates** from `mac_enforce_bit_length` / `enforce_bit_length` —
   chunks allocated as **Committed** (remainder ≈ **1,552**)

Core gadget: `video/src/encode/constraints.rs` (`enforce_bit_length` → committed chunks of
width `log2(table_len)` = 8). Used heavily in `quant_core` and `MatrixVar::regroup`.

**EditOnly:** still `set_table(0..256)` so Nova’s driver doesn’t break, but encode never
calls `enforce_bit_length` — only pixel (and some edit) committed vars are queries.

### Scaling check

At 256 blocks: `≈ 2320 × 256 + 256 = 594,176` committed — matches Phase 0.

---

## Why this blocks a naive NeutronNova port

Spartan2 NeutronNova expects circuits as **`SpartanCircuit`** in **bellpepper**, with:

- `shared` / `precommitted` / `synthesize` witness split
- Optional `num_challenges` for commit→squeeze→aux (FS challenges)

Eva's lookup is:

- Implemented directly in **ark-r1cs-std** constraint generation
- Tightly coupled to Nova's **committed witness** layout (`num_committed_variables`)
- Applied **after** step circuit synthesis via `build_histo` + `generate_lookup_constraints`

There is no drop-in mapping to Spartan2's API without reimplementing the lookup in bellpepper.

## Port options (Phase 2 evaluation)

### Option A: Reimplement LogUp in bellpepper ← **in progress**

- Rewrite `LookupArgument` using bellpepper gadgets
- Integrate histogram building into circuit witness
- **Effort:** high; **fidelity:** highest

### Option B: Lasso / zero-check reduction (Spartan2-native)

- NeutronNova paper reduces lookups to zero-check instances
- Spartan2 may expose patterns for this (see Jolt/Lasso lineage)
- **Effort:** very high; **potential gain:** best asymptotics if it works

### Option C: Hybrid — keep Nova step, Spartan decider only ← **landed in `video`**

- Continue Nova folding for step + lookups
- Two parallel final SNARKs on the same `DeciderEthCircuit`
- **Effort:** medium; **does not** capture NeutronNova folding wins
- **Status:** `video::decider::{Decider, SpartanDecider}` — see [`OPTION_C_TRANSPARENT.md`](./OPTION_C_TRANSPARENT.md)
  - Groth16: full `DeciderEth` (primary + CycleFold + sigma)
  - Spartan: **same statement** via ark → matrix Spartan (primary-only path removed)
### Option D: R1CS export bridge (research)

- Export arkworks R1CS matrices → Spartan2 `R1CSShape`
- Prove exported R1CS without re-synthesis
- **Effort:** medium for single-instance; **IVC/streaming:** unclear

## Phase 2 progress (Option A)

### Done (Phase 2.0)

| Item | Location |
|------|----------|
| Witness layout documented | this file |
| Bellpepper LogUp gadgets | `comparison/src/bellpepper/lookup.rs` |
| `SpartanCircuit` wrapper | `comparison/src/bellpepper/step.rs` |
| NeutronNova smoke example | `comparison/examples/phase2_lookup_smoke.rs` |
| Results | [`results/phase2.md`](./results/phase2.md) |

**What Phase 2.0 proves:** LogUp identity (table 0..255, synthetic queries) synthesizes in
bellpepper, constraint count matches `Q + T + 1` (+1 public IO), and NeutronNova
setup → prep_prove → prove → verify succeeds on BN254 for:

- `NUM_QUERIES=16` (unit / CI)
- `NUM_QUERIES=384` (**1 macroblock of pixels**)
- `NUM_QUERIES=2320` (**full 1-MB NoOp committed-query scale**)

### Known limitations (follow-ups)

1. **Not full Eva step** — no encode / Griffin / AugmentedFCircuit; lookup-only.
2. **Fixed witness challenge `c`** — not `Poseidon(cmQ)` / Spartan2 `num_challenges`.
   Trying `num_challenges=1` caused NeutronNova verify failures (`Challenges do not match`).
3. **All witness in `synthesize`** — Phase 0 found BN254 verify fragile when only
   `precommitted` holds constraints; queries moved to `precommitted` is deferred until
   FS challenge binding works.

### Recommended next order

1. Revisit `precommitted` + `num_challenges=1` (FS `c`) on BN254
2. Port `enforce_bit_length` / pixel commit gadgets into bellpepper
3. Integrate into a full `SpartanCircuit` Eva step prototype (target ~6,117 constraints)
## Files

| File | Role |
|------|------|
| `folding-schemes/src/frontend/mod.rs` | Current LogUp implementation (reference) |
| `video/src/lib.rs` | Macroblock constraints + lookup table setup |
| `folding-schemes/src/folding/nova/circuits.rs` | AugmentedFCircuit integration |
| `comparison/src/bellpepper/lookup.rs` | bellpepper LogUp (Phase 2.0) |
| `comparison/src/bellpepper/step.rs` | LogUp as `SpartanCircuit` (Phase 2.0) |
| `comparison/examples/phase2_lookup_smoke.rs` | NeutronNova smoke harness |

## Open questions

- Can Spartan2 `is_small=true` apply once queries stay in 0..255 **and** inverses are
  isolated from the small-path commitment? (Currently `is_small=false` because inverses
  are full field elements in the same witness region.)
- Does NeutronNova batch proving align with Eva's streaming `prove_step`, or must we
  batch macroblock steps?
- What is the peak RAM of lookup constraints at 1.43M rows in Spartan2 vs Nova?

## References

- Eva LogUp: `folding-schemes/src/frontend/mod.rs`
- Lasso: [ePrint 2022/1380](https://eprint.iacr.org/2022/1380)
- NeutronNova reductions: [ePrint 2024/1606](https://eprint.iacr.org/2024/1606) §1.2
