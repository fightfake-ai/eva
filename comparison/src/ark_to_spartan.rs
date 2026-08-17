//! Convert an arkworks (winderica) R1CS + assignment into Spartan2 matrix form
//! and prove it with a **standalone** transparent Spartan (relaxed, `u=1`, `E=0`).
//!
//! ## Why this exists
//!
//! Eva’s production decider is `DeciderEthCircuit` (~7M constraints at QUICK scale).
//! Re-encoding that circuit as a bellpepper `SpartanCircuit` (~1 enforce/row or worse)
//! is not practical. Spartan2 0.9 keeps the matrix API private upstream; we vendor a
//! patched copy (`third_party/spartan2`) and prove the **same** ark R1CS Groth16 uses.
//!
//! ## Variable layout remap
//!
//! Arkworks / Eva column order (see `ConstraintSystem::make_row`):
//! ```text
//! z_ark = [ ONE | X... | Q (committed) | W (witness) ]
//! ```
//!
//! Spartan2 R1CS column order:
//! ```text
//! z_spartan = [ W_priv... | ONE_or_u | X... ]
//! ```
//! where `W_priv = Q || W` (committed then witness).
//!
//! ## Standalone FS binding
//!
//! Upstream `RelaxedR1CSSpartanProof` does not absorb `comm_W`/`comm_E` (NIFS outer
//! protocol is assumed). Here we absorb both commitments before prove/verify so the
//! proof is sound as a standalone transparent SNARK for the committed witness.

use ark_ff::{BigInteger, PrimeField as ArkPrimeField};
use ark_relations::r1cs::{ConstraintMatrices, ConstraintSystemRef};
use ff::{Field, PrimeField as FfPrimeField};
use serde::Serialize;
use spartan2::provider::Bn254Engine;
use spartan2::r1cs::{R1CSShape, RelaxedR1CSInstance, RelaxedR1CSWitness, SparseMatrix};
use spartan2::spartan_relaxed::RelaxedR1CSSpartanProof;
use spartan2::traits::transcript::TranscriptEngineTrait;
use spartan2::traits::Engine;

type E = Bn254Engine;

/// Result of converting an ark ConstraintSystem into a Spartan shape + witness.
pub struct ArkR1CSConversion {
    pub shape: R1CSShape<E>,
    pub W: Vec<<E as Engine>::Scalar>,
    pub X: Vec<<E as Engine>::Scalar>,
    pub num_constraints_unpadded: usize,
    pub num_constraints_padded: usize,
    pub num_vars: usize,
    pub num_io: usize,
    pub num_committed: usize,
    pub num_witness: usize,
}

fn ark_fr_to_spartan<F: ArkPrimeField>(x: F) -> <E as Engine>::Scalar {
    let bytes = x.into_bigint().to_bytes_le();
    let mut repr = <<E as Engine>::Scalar as FfPrimeField>::Repr::default();
    let dst: &mut [u8] = repr.as_mut();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
    <E as Engine>::Scalar::from_repr(repr).unwrap()
}

/// Map an arkworks column index into a Spartan column index.
fn map_ark_col_to_spartan(
    ark_col: usize,
    num_instance: usize,
    num_committed: usize,
    num_vars: usize,
) -> usize {
    // Spartan: [0 .. num_vars) = W_priv, num_vars = ONE, num_vars+1 .. = X
    if ark_col == 0 {
        num_vars // ONE
    } else if ark_col < num_instance {
        // X[ark_col - 1] sits at num_vars + ark_col
        num_vars + ark_col
    } else if ark_col < num_instance + num_committed {
        ark_col - num_instance // committed → front of W_priv
    } else {
        num_committed + (ark_col - num_instance - num_committed) // witness
    }
}

fn convert_matrix<F: ArkPrimeField>(
    rows: &[Vec<(F, usize)>],
    num_rows_padded: usize,
    num_cols: usize,
    num_instance: usize,
    num_committed: usize,
    num_vars: usize,
) -> SparseMatrix<<E as Engine>::Scalar> {
    let mut coo: Vec<(usize, usize, <E as Engine>::Scalar)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut mapped: Vec<(usize, <E as Engine>::Scalar)> = row
            .iter()
            .map(|(coeff, col)| {
                (
                    map_ark_col_to_spartan(*col, num_instance, num_committed, num_vars),
                    ark_fr_to_spartan(*coeff),
                )
            })
            .collect();
        // SparseMatrix::new requires strictly increasing columns within a row.
        mapped.sort_by_key(|(c, _)| *c);
        // Merge duplicate columns (possible after remap? shouldn't happen, but safe).
        let mut merged: Vec<(usize, <E as Engine>::Scalar)> = Vec::new();
        for (c, v) in mapped {
            if let Some(last) = merged.last_mut() {
                if last.0 == c {
                    last.1 += v;
                    continue;
                }
            }
            merged.push((c, v));
        }
        for (c, v) in merged {
            if v != <E as Engine>::Scalar::ZERO {
                coo.push((r, c, v));
            }
        }
    }
    SparseMatrix::new(&coo, num_rows_padded, num_cols)
}

/// Extract Spartan shape + `(W, X)` from a finalized ark ConstraintSystem that
/// already has assignments (Prove mode with matrices).
pub fn convert_ark_cs<F: ArkPrimeField>(
    cs: &ConstraintSystemRef<F>,
) -> Result<ArkR1CSConversion, String> {
    cs.finalize();
    let borrow = cs
        .borrow()
        .ok_or_else(|| "CS borrow failed".to_string())?;
    if !borrow.is_satisfied().map_err(|e| format!("is_satisfied: {e}"))? {
        return Err("ark ConstraintSystem is not satisfied".into());
    }

    let matrices: ConstraintMatrices<F> = borrow
        .to_matrices()
        .ok_or_else(|| "to_matrices returned None (construct_matrices=false?)".to_string())?;

    let num_instance = matrices.num_instance_variables;
    let num_witness = matrices.num_witness_variables;
    let num_committed = borrow.num_committed_variables;
    let num_cons = matrices.num_constraints;
    if num_instance == 0 {
        return Err("expected at least the constant ONE instance var".into());
    }
    if borrow.instance_assignment.len() != num_instance {
        return Err(format!(
            "instance_assignment len {} != num_instance {}",
            borrow.instance_assignment.len(),
            num_instance
        ));
    }
    if borrow.committed_assignment.len() != num_committed {
        return Err(format!(
            "committed_assignment len {} != num_committed {}",
            borrow.committed_assignment.len(),
            num_committed
        ));
    }
    if borrow.witness_assignment.len() != num_witness {
        return Err(format!(
            "witness_assignment len {} != num_witness {}",
            borrow.witness_assignment.len(),
            num_witness
        ));
    }

    let num_io = num_instance - 1;
    let num_vars = num_committed + num_witness;
    let num_cons_padded = num_cons.next_power_of_two().max(1);
    let num_cols = num_vars + 1 + num_io;

    let A = convert_matrix(
        &matrices.a,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );
    let B = convert_matrix(
        &matrices.b,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );
    let C = convert_matrix(
        &matrices.c,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );

    let shape = R1CSShape::<E>::new(num_cons_padded, num_vars, num_io, A, B, C)
        .map_err(|e| format!("R1CSShape::new: {e}"))?;

    // W_priv = committed || witness
    let mut W: Vec<<E as Engine>::Scalar> = borrow
        .committed_assignment
        .iter()
        .chain(borrow.witness_assignment.iter())
        .map(|x| ark_fr_to_spartan(*x))
        .collect();
    W.resize(num_vars, <E as Engine>::Scalar::ZERO);

    let X: Vec<<E as Engine>::Scalar> = borrow.instance_assignment[1..]
        .iter()
        .map(|x| ark_fr_to_spartan(*x))
        .collect();

    // Sanity: Az ∘ Bz = Cz with u=1
    {
        let z: Vec<_> = [W.clone(), vec![<E as Engine>::Scalar::ONE], X.clone()].concat();
        let (az, bz, cz) = shape
            .multiply_vec(&z)
            .map_err(|e| format!("multiply_vec: {e}"))?;
        for i in 0..num_cons {
            if az[i] * bz[i] != cz[i] {
                return Err(format!(
                    "converted R1CS unsat at row {i}: Az*Bz != Cz (after ark→Spartan remap)"
                ));
            }
        }
    }

    Ok(ArkR1CSConversion {
        shape,
        W,
        X,
        num_constraints_unpadded: num_cons,
        num_constraints_padded: num_cons_padded,
        num_vars,
        num_io,
        num_committed,
        num_witness,
    })
}

/// Transparent proof bundle for a converted ark R1CS (standard instance as relaxed).
#[derive(Clone, Serialize)]
pub struct TransparentDeciderProof {
    pub proof: RelaxedR1CSSpartanProof<E>,
    pub U: RelaxedR1CSInstance<E>,
}

/// Timings for the full transparent path.
#[derive(Clone, Debug, Default)]
pub struct TransparentProveTimings {
    pub synthesize_ms: f64,
    pub convert_ms: f64,
    pub setup_ms: f64,
    pub prove_ms: f64,
    pub verify_ms: f64,
    pub proof_bytes: usize,
    pub num_constraints: usize,
    pub num_constraints_padded: usize,
    pub num_vars: usize,
    pub num_io: usize,
}

fn absorb_instance_commitments(
    transcript: &mut <E as Engine>::TE,
    U: &RelaxedR1CSInstance<E>,
) {
    U.absorb_commitments(transcript);
}

/// Setup + prove + verify a converted R1CS as standard R1CS (`u=1`, `E=0`).
pub fn prove_verify_transparent(
    conv: ArkR1CSConversion,
) -> Result<(TransparentDeciderProof, TransparentProveTimings), String> {
    use std::time::Instant;

    let mut timings = TransparentProveTimings {
        num_constraints: conv.num_constraints_unpadded,
        num_constraints_padded: conv.num_constraints_padded,
        num_vars: conv.num_vars,
        num_io: conv.num_io,
        ..Default::default()
    };

    let t = Instant::now();
    let (ck, vk_ee) = conv.shape.commitment_key();
    timings.setup_ms = t.elapsed().as_secs_f64() * 1000.0;

    let E_zeros = vec![<E as Engine>::Scalar::ZERO; conv.num_constraints_padded];
    let (W, U) = RelaxedR1CSWitness::<E>::from_assignments(
        &ck,
        &conv.shape,
        conv.W,
        E_zeros,
        <E as Engine>::Scalar::ONE,
        conv.X,
    )
    .map_err(|e| format!("from_assignments: {e}"))?;

    conv.shape
        .is_sat_relaxed(&ck, &U, &W)
        .map_err(|e| format!("is_sat_relaxed: {e}"))?;

    let t = Instant::now();
    let mut transcript_p = <E as Engine>::TE::new(b"EvaTransparentDecider");
    absorb_instance_commitments(&mut transcript_p, &U);
    let proof = RelaxedR1CSSpartanProof::<E>::prove(
        &conv.shape,
        &ck,
        U.u(),
        U.X(),
        &W,
        &mut transcript_p,
    )
    .map_err(|e| format!("Spartan prove: {e}"))?;
    timings.prove_ms = t.elapsed().as_secs_f64() * 1000.0;

    let bundle = TransparentDeciderProof {
        proof: proof.clone(),
        U: U.clone(),
    };
    timings.proof_bytes = bincode::serialize(&bundle)
        .map(|b| b.len())
        .map_err(|e| format!("bincode: {e}"))?;

    let t = Instant::now();
    let mut transcript_v = <E as Engine>::TE::new(b"EvaTransparentDecider");
    absorb_instance_commitments(&mut transcript_v, &bundle.U);
    bundle
        .proof
        .verify(&conv.shape, &vk_ee, &bundle.U, &mut transcript_v)
        .map_err(|e| format!("Spartan verify: {e}"))?;
    timings.verify_ms = t.elapsed().as_secs_f64() * 1000.0;

    Ok((bundle, timings))
}

/// Synthesize an ark `ConstraintSynthesizer`, convert, prove, verify.
pub fn prove_ark_circuit<C, F>(
    circuit: C,
) -> Result<(TransparentDeciderProof, TransparentProveTimings), String>
where
    C: ark_relations::r1cs::ConstraintSynthesizer<F>,
    F: ArkPrimeField,
{
    use std::time::Instant;
    use ark_relations::r1cs::ConstraintSystem;

    let t = Instant::now();
    let cs = ConstraintSystem::<F>::new_ref();
    circuit
        .generate_constraints(cs.clone())
        .map_err(|e| format!("generate_constraints: {e}"))?;
    let synthesize_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let conv = convert_ark_cs(&cs)?;
    let convert_ms = t.elapsed().as_secs_f64() * 1000.0;

    let (proof, mut timings) = prove_verify_transparent(conv)?;
    timings.synthesize_ms = synthesize_ms;
    timings.convert_ms = convert_ms;
    Ok((proof, timings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr;
    use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
    use ark_r1cs_std::fields::fp::FpVar;
    use ark_r1cs_std::prelude::*;

    struct TinyMul {
        a: Fr,
        b: Fr,
        c: Fr,
    }

    impl ConstraintSynthesizer<Fr> for TinyMul {
        fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
            let a = FpVar::new_witness(cs.clone(), || Ok(self.a))?;
            let b = FpVar::new_witness(cs.clone(), || Ok(self.b))?;
            let c = FpVar::new_input(cs, || Ok(self.c))?;
            let prod = &a * &b;
            prod.enforce_equal(&c)?;
            Ok(())
        }
    }

    #[test]
    fn tiny_ark_circuit_proves_transparent() {
        let circuit = TinyMul {
            a: Fr::from(3u64),
            b: Fr::from(5u64),
            c: Fr::from(15u64),
        };
        let (_proof, t) = prove_ark_circuit(circuit).expect("prove");
        assert!(t.num_constraints > 0);
        assert!(t.proof_bytes > 0);
    }
}
