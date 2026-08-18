# Vendored Spartan2 0.9.0

Copied from crates.io `spartan2-0.9.0` with minimal patches so Eva can prove an
**external** arkworks R1CS (`DeciderEthCircuit`) without re-encoding it as a
bellpepper `SpartanCircuit`.

Used by `video::decider::SpartanDecider`.

## Patches vs upstream

1. `pub mod r1cs` (was private) — expose matrix-native types.
2. `pub use SparseMatrix` + `SparseMatrix::new` always available (was `#[cfg(test)]`).
3. `RelaxedR1CSWitness::from_assignments` — build `(W, E, U)` from vectors.
4. `R1CSShape::commitment_key` sizes PCS for `max(num_vars, num_cons)` so `E`
   openings work when there are more constraints than variables.

## Standalone soundness wrapper

Upstream `RelaxedR1CSSpartanProof` intentionally does **not** absorb
`comm_W` / `comm_E` (it is meant to sit under NIFS). Eva’s transparent
decider absorbs those commitments into the Fiat–Shamir transcript **before**
`prove` / `verify` (`video/src/decider/spartan.rs`).
