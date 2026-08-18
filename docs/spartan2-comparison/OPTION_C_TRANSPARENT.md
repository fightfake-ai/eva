# Dual deciders (Groth16 + transparent Spartan)

Eva’s final compression of a finished Nova IVC run is [`DeciderEthCircuit`](../../video/src/decider/mod.rs):
primary relaxed R1CS + CycleFold + device sigma, in one BN254 R1CS.

Two backends prove **that same circuit**:

| API | Setup | Proof | Verify | Where |
|-----|--------|-------|--------|--------|
| [`Decider`](../../video/src/decider/mod.rs) | Groth16 (trusted) | ~256 B | ~ms | production / EVM |
| [`SpartanDecider`](../../video/src/decider/spartan.rs) | Hyrax CRS (transparent) | hundreds of KB | seconds at QUICK | offline / no toxic waste |

```
Nova IVC (unchanged)
        │
        ▼
 DeciderEthCircuit::from_nova
        │
   ┌────┴────┐
   ▼         ▼
Decider    SpartanDecider
(Groth16)  (matrix Spartan)
```

Primary-only Spartan (bellpepper re-encoding of the folded step R1CS) was an
incomplete spike and has been **removed**.

## Run

```bash
export DATA_PATH=/path/to/data_parsed
QUICK=1 cargo run --release -p video --example edit_lossless_decider
QUICK=1 DECIDER=spartan cargo run --release -p video --example edit_lossless_decider
```

`SpartanDecider` is behind `--features spartan` (on in `video`’s default features).
`wasm-prove-spike` keeps `default-features = false` so it does not pull Spartan2.

## How Spartan proves the ark circuit

1. Synthesize `DeciderEthCircuit` into arkworks `ConstraintSystem`.
2. Remap columns `[ONE | X | Q | W]` → `[Q ‖ W | ONE | X]`.
3. Pad constraint count to the next power of two.
4. Prove standard R1CS as relaxed (`u=1`, `E=0`) with vendored
   `RelaxedR1CSSpartanProof`, after absorbing `comm_W` / `comm_E`.

Measured QUICK (`4` blocks × `4` steps): ~7.0M constraints, prove ~79 s,
verify ~3.7 s, proof ~370 KB. See [results/phase3_option_c.md](./results/phase3_option_c.md).
