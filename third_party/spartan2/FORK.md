# Vendored Spartan2 0.9.0 (Eva Option C)

Copied from crates.io `spartan2-0.9.0` with minimal patches so comparison
can prove an **external** arkworks R1CS (the full `DeciderEthCircuit`) without
re-encoding it as a bellpepper `SpartanCircuit`.

## Patches vs upstream

1. `pub mod r1cs` (was private) — expose matrix-native types.
2. `pub use SparseMatrix` + `SparseMatrix::new` always available (was `#[cfg(test)]`).
3. `RelaxedR1CSWitness::from_assignments` — build `(W, E, U)` from vectors.
4. `R1CSShape::commitment_key` sizes PCS for `max(num_vars, num_cons)` so `E`
   openings work when there are more constraints than variables.

## Standalone soundness wrapper

Upstream `RelaxedR1CSSpartanProof` intentionally does **not** absorb
`comm_W` / `comm_E` (it is meant to sit under NIFS). For Eva’s transparent
decider we absorb those commitments into the Fiat–Shamir transcript **before**
`prove` / `verify` (see `comparison/src/ark_to_spartan.rs`).
