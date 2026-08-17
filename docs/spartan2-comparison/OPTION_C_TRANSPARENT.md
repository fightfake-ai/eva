# Option C — Complete transparent decider

This document explains the **complete transparent-setup** final proof for Eva:
same cryptographic statement as today’s Groth16 `DeciderEthCircuit`, without a
trusted setup ceremony.

Read with [ARCHITECTURE.md](./ARCHITECTURE.md), [LOOKUPS.md](./LOOKUPS.md), and
[results/phase3_option_c.md](./results/phase3_option_c.md).

---

## 1. What “complete” means

After Nova IVC (steps + LogUp lookups + CycleFold on Grumpkin), Eva finishes with a
**decider** that compresses the final folded state into one succinct proof.

### Groth16 decider (production today)

`video/src/decider.rs` → `DeciderEthCircuit` proves, in one BN254 R1CS:

| Check | Role |
|-------|------|
| **Primary relaxed R1CS** | Final folded Nova instance `Az∘Bz = u·Cz+E` on BN254 |
| **CycleFold** | Non-native verification of the Grumpkin CycleFold running instance (commitment folding) |
| **Device sigma** | Schnorr-style PoK of device `sk` binding `h1` (original pixel hash) to `vk` |

Verifier public inputs include step count `i`, `z_0`, `h2`, device `vk` coords, commitment
limbs, fold challenge `r`, etc. Proof size ≈ **256 bytes**. Setup is **trusted**
(Groth16 toxic waste).

### Incomplete Spartan path (earlier spike)

`SPARTAN_MODE=primary` re-encodes **only** the primary matrices as a bellpepper
`RelaxedR1CSCheckCircuit` (~5 constraints/row) and proves with `SpartanZkSNARK`.

That is transparent and fast at QUICK scale, but it does **not** check CycleFold or
sigma — a verifier of that proof alone does not get the same assurance as Groth16.

### Complete transparent path (this work)

`SPARTAN_MODE=full` (default):

1. Build the **same** `DeciderEthCircuit` via `from_nova` (identical witness / constraints
   as Groth16 prove).
2. Synthesize into arkworks `ConstraintSystem` (winderica fork with committed vars).
3. Remap columns to Spartan2’s `z = (W_priv ‖ 1 ‖ X)` layout.
4. Prove with matrix-native `RelaxedR1CSSpartanProof` as a **standard** R1CS
   (`u = 1`, `E = 0`), after absorbing `comm_W` / `comm_E` into Fiat–Shamir.

So the **statement** matches Groth16. The **proof system** is transparent (Hyrax PCS +
sum-check). Proofs are larger and verify slower than Groth16; setup has no toxic waste.

```
Nova IVC (unchanged)
        │
        ▼
 DeciderEthCircuit::from_nova   ←── same circuit for both backends
        │
   ┌────┴────┐
   ▼         ▼
Groth16    ark CS → remap → Spartan matrix prove
(trusted)              (transparent)
```

---

## 2. Why not re-encode Decider in bellpepper?

At QUICK scale (`BLOCKS_PER_STEP=4`, `NUM_STEPS=4`):

| Metric | Approx. |
|--------|---------|
| DeciderEth constraints | **~7.0M** |
| Witness vars | **~6.9M** |
| Committed vars | **~175k** |

A 1-constraint-per-row bellpepper wrapper would still be ~7M constraints; a 5× relaxed
check would be ~35M. That is far heavier than proving the native matrices once.

Spartan2 0.9 keeps `mod r1cs` **private**, so matrix proving was unreachable from crates.io
without a fork. We vendor `third_party/spartan2` with small patches — see
[`third_party/spartan2/FORK.md`](../../third_party/spartan2/FORK.md).

---

## 3. Ark → Spartan column remap

Eva / arkworks (winderica) `make_row` ordering:

```text
z_ark = [ ONE | public X... | committed Q... | witness W... ]
```

Spartan2 `R1CSShape` expects:

```text
z_spartan = [ W_priv... | ONE_or_u | X... ]
W_priv    = Q ‖ W
```

Conversion lives in `comparison/src/ark_to_spartan.rs`:

- `map_ark_col_to_spartan` remaps every nonzero matrix entry.
- Constraint count is padded to the next power of two (sum-check requirement).
- After convert, we check `Az∘Bz = Cz` natively before proving.

---

## 4. Standalone soundness (commitment binding)

Upstream comment on `RelaxedR1CSSpartanProof`:

> This proof does NOT absorb `comm_W`/`comm_E` into its transcript — only `(u, X)`.
> It is sound only when used within an outer protocol (e.g. NIFS).

For a **standalone** transparent decider we therefore:

```text
transcript ← "EvaTransparentDecider"
absorb(comm_W); absorb(comm_E);     // our wrapper
prove/verify as upstream (absorbs u, X, then sum-check)
```

Implemented as `RelaxedR1CSInstance::absorb_commitments` + callers in
`prove_verify_transparent`.

**ZK note:** this matrix path is the non-ZK relaxed Spartan (witness openings are
deterministic given the commitment blinds). Production on-chain use may still prefer
Groth16’s small proofs, or a future ZK wrapper (`SpartanZkSNARK`) if proof size/privacy
require it. The goal here is **transparent setup + complete statement**.

---

## 5. Device sigma (what the circuit checks)

Sigma is a Schnorr-style proof of knowledge of `sk` for `vk = sk · G` on Grumpkin,
bound to `h1 = z_i[0]` (hash of the authenticated original):

```text
r ← random
R = r·G,  rx = R.x
e = Poseidon(rx, vk.x, vk.y, h1)
s = r + sk·e
Circuit:  s·G - e·vk  has x-coordinate rx
```

Allocated in `DeciderEthCircuit::generate_constraints` (`sigma_r`, `sigma_s` bits, `vk`
as public input). The full Spartan path includes these constraints automatically because
it proves the whole Decider R1CS.

---

## 6. CycleFold in the decider

CycleFold’s running instance lives on **Grumpkin**. The decider verifies it with
**non-native** field gadgets on BN254 (expensive — majority of the ~7M constraints).
Primary Nova folding is native BN254.

Proving the full Decider R1CS therefore covers CycleFold without a separate Grumpkin
Spartan instance.

---

## 7. How to run

```bash
# Transparent full decider only (no Groth16 ceremony)
SKIP_G16=1 SPARTAN_MODE=full \
  cargo run --release -p comparison --example phase3_option_c

# Dual path: Groth16 + full Spartan (slow: G16 setup ~2 min at QUICK)
SPARTAN_MODE=full \
  cargo run --release -p comparison --example phase3_option_c

# Legacy incomplete primary-only Spartan
SKIP_G16=1 SPARTAN_MODE=primary \
  cargo run --release -p comparison --example phase3_option_c

BLOCKS_PER_STEP=4 NUM_STEPS=4   # defaults; need ≥4 steps so U.cmE ≠ ∞
```

Constraint count only:

```bash
cargo run --release -p comparison --example count_decider_constraints
```

---

## 8. Code map

| Path | Role |
|------|------|
| `comparison/src/option_c.rs` | Dual-path runner; `SPARTAN_MODE` |
| `comparison/src/ark_to_spartan.rs` | Ark CS → Spartan shape; prove/verify |
| `comparison/src/bellpepper/relaxed_check.rs` | Primary-only bellpepper path |
| `third_party/spartan2/` | Patched Spartan2 0.9 (public `r1cs`) |
| `video/src/decider.rs` | Production Groth16 decider + circuit |

---

## 9. Measured trade-offs (QUICK: 4 blocks × 4 steps, 2026-08-17)

| | Groth16 | Spartan full (transparent) |
|--|---------|----------------------------|
| Setup | ~116 s (trusted) | ~23 ms (Hyrax CRS) |
| Statement | Full DeciderEth | **Same** (~7.0M cons) |
| Prove | ~113 s | ~79 s (synth 13 + convert 59 + core 7) |
| Verify | ~4 ms | ~3.7 s |
| Proof size | **256 B** | **~370 KB** |

Spartan full is competitive on prover wall-clock at this scale (no ceremony) but loses badly
on proof size and verify — fine for transparent offline / side-channel verification, not a
drop-in EVM replacement for Groth16.

Record / refresh numbers in [results/phase3_option_c.md](./results/phase3_option_c.md).
