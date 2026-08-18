//! Transparent Spartan backend for [`super::DeciderEthCircuit`].
//!
//! Synthesizes the same arkworks circuit Groth16 uses, remaps columns to Spartan2
//! layout, and proves with matrix-native `RelaxedR1CSSpartanProof` (`u = 1`, `E = 0`).
//!
//! Ark / Eva column order: `z = [ ONE | X | committed Q | witness W ]`  
//! Spartan2 column order:  `z = [ Q ‖ W | ONE | X ]`

use ark_ff::{BigInteger, PrimeField as ArkPrimeField};
use ark_relations::r1cs::{ConstraintMatrices, ConstraintSynthesizer, ConstraintSystem, ConstraintSystemRef};
use ff::{Field, PrimeField as FfPrimeField};
use serde::{Deserialize, Serialize};
use spartan2::provider::Bn254Engine;
use spartan2::r1cs::{R1CSShape, RelaxedR1CSInstance, RelaxedR1CSWitness, SparseMatrix};
use spartan2::spartan_relaxed::RelaxedR1CSSpartanProof;
use spartan2::traits::pcs::PCSEngineTrait;
use spartan2::traits::transcript::TranscriptEngineTrait;
use spartan2::traits::Engine;

use folding_schemes::Error;

type E = Bn254Engine;
type Scalar = <E as Engine>::Scalar;
type PcsVk = <<E as Engine>::PCS as PCSEngineTrait<E>>::VerifierKey;

const TRANSCRIPT_LABEL: &[u8] = b"EvaTransparentDecider";

/// Proof of `DeciderEthCircuit` under transparent Spartan.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpartanProof {
    pub proof: RelaxedR1CSSpartanProof<E>,
    pub instance: RelaxedR1CSInstance<E>,
}

/// Verifying material for [`SpartanDecider`] (circuit shape + Hyrax VK).
///
/// Unlike Groth16, this is linear in the R1CS size. Suitable for offline
/// verification, not a drop-in EVM VK.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpartanVerifierKey {
    pub shape: R1CSShape<E>,
    pub pcs_vk: PcsVk,
    pub num_constraints: usize,
    pub num_constraints_padded: usize,
    pub num_vars: usize,
    pub num_io: usize,
}

struct Conversion {
    shape: R1CSShape<E>,
    w: Vec<Scalar>,
    x: Vec<Scalar>,
    num_constraints: usize,
    num_constraints_padded: usize,
    num_vars: usize,
    num_io: usize,
}

fn ark_fr_to_spartan<F: ArkPrimeField>(x: F) -> Scalar {
    let bytes = x.into_bigint().to_bytes_le();
    let mut repr = <Scalar as FfPrimeField>::Repr::default();
    let dst: &mut [u8] = repr.as_mut();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
    Scalar::from_repr(repr).unwrap()
}

fn map_ark_col_to_spartan(
    ark_col: usize,
    num_instance: usize,
    num_committed: usize,
    num_vars: usize,
) -> usize {
    if ark_col == 0 {
        num_vars
    } else if ark_col < num_instance {
        num_vars + ark_col
    } else if ark_col < num_instance + num_committed {
        ark_col - num_instance
    } else {
        num_committed + (ark_col - num_instance - num_committed)
    }
}

fn convert_matrix<F: ArkPrimeField>(
    rows: &[Vec<(F, usize)>],
    num_rows_padded: usize,
    num_cols: usize,
    num_instance: usize,
    num_committed: usize,
    num_vars: usize,
) -> SparseMatrix<Scalar> {
    let mut coo: Vec<(usize, usize, Scalar)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut mapped: Vec<(usize, Scalar)> = row
            .iter()
            .map(|(coeff, col)| {
                (
                    map_ark_col_to_spartan(*col, num_instance, num_committed, num_vars),
                    ark_fr_to_spartan(*coeff),
                )
            })
            .collect();
        mapped.sort_by_key(|(c, _)| *c);
        let mut merged: Vec<(usize, Scalar)> = Vec::new();
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
            if v != Scalar::ZERO {
                coo.push((r, c, v));
            }
        }
    }
    SparseMatrix::new(&coo, num_rows_padded, num_cols)
}

fn convert_ark_cs<F: ArkPrimeField>(cs: &ConstraintSystemRef<F>) -> Result<Conversion, Error> {
    cs.finalize();
    let borrow = cs
        .borrow()
        .ok_or_else(|| Error::Other("CS borrow failed".into()))?;
    if !borrow
        .is_satisfied()
        .map_err(|e| Error::Other(format!("is_satisfied: {e}")))?
    {
        return Err(Error::NotSatisfied);
    }

    let matrices: ConstraintMatrices<F> = borrow
        .to_matrices()
        .ok_or_else(|| Error::Other("to_matrices returned None".into()))?;

    let num_instance = matrices.num_instance_variables;
    let num_witness = matrices.num_witness_variables;
    let num_committed = borrow.num_committed_variables;
    let num_cons = matrices.num_constraints;
    if num_instance == 0 {
        return Err(Error::Other(
            "expected at least the constant ONE instance var".into(),
        ));
    }
    if borrow.instance_assignment.len() != num_instance
        || borrow.committed_assignment.len() != num_committed
        || borrow.witness_assignment.len() != num_witness
    {
        return Err(Error::Other("assignment length mismatch".into()));
    }

    let num_io = num_instance - 1;
    let num_vars = num_committed + num_witness;
    let num_cons_padded = num_cons.next_power_of_two().max(1);
    let num_cols = num_vars + 1 + num_io;

    let a = convert_matrix(
        &matrices.a,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );
    let b = convert_matrix(
        &matrices.b,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );
    let c = convert_matrix(
        &matrices.c,
        num_cons_padded,
        num_cols,
        num_instance,
        num_committed,
        num_vars,
    );

    let shape = R1CSShape::<E>::new(num_cons_padded, num_vars, num_io, a, b, c)
        .map_err(|e| Error::Other(format!("R1CSShape::new: {e}")))?;

    let mut w: Vec<Scalar> = borrow
        .committed_assignment
        .iter()
        .chain(borrow.witness_assignment.iter())
        .map(|x| ark_fr_to_spartan(*x))
        .collect();
    w.resize(num_vars, Scalar::ZERO);

    let x: Vec<Scalar> = borrow.instance_assignment[1..]
        .iter()
        .map(|v| ark_fr_to_spartan(*v))
        .collect();

    {
        let z: Vec<_> = [w.clone(), vec![Scalar::ONE], x.clone()].concat();
        let (az, bz, cz) = shape
            .multiply_vec(&z)
            .map_err(|e| Error::Other(format!("multiply_vec: {e}")))?;
        for i in 0..num_cons {
            if az[i] * bz[i] != cz[i] {
                return Err(Error::Other(format!(
                    "converted R1CS unsat at row {i} after ark→Spartan remap"
                )));
            }
        }
    }

    Ok(Conversion {
        shape,
        w,
        x,
        num_constraints: num_cons,
        num_constraints_padded: num_cons_padded,
        num_vars,
        num_io,
    })
}

/// Transparent final SNARK for [`super::DeciderEthCircuit`].
pub struct SpartanDecider;

impl SpartanDecider {
    /// Prove `circuit` (typically `DeciderEthCircuit::from_nova(...)`).
    pub fn prove<C, F>(circuit: C) -> Result<(SpartanProof, SpartanVerifierKey), Error>
    where
        C: ConstraintSynthesizer<F>,
        F: ArkPrimeField,
    {
        let cs = ConstraintSystem::<F>::new_ref();
        circuit.generate_constraints(cs.clone())?;
        let conv = convert_ark_cs(&cs)?;

        let (ck, pcs_vk) = conv.shape.commitment_key();
        let e_zeros = vec![Scalar::ZERO; conv.num_constraints_padded];
        let (w, u) = RelaxedR1CSWitness::<E>::from_assignments(
            &ck,
            &conv.shape,
            conv.w,
            e_zeros,
            Scalar::ONE,
            conv.x,
        )
        .map_err(|e| Error::Other(format!("from_assignments: {e}")))?;

        conv.shape
            .is_sat_relaxed(&ck, &u, &w)
            .map_err(|e| Error::Other(format!("is_sat_relaxed: {e}")))?;

        let mut transcript = <E as Engine>::TE::new(TRANSCRIPT_LABEL);
        u.absorb_commitments(&mut transcript);
        let proof = RelaxedR1CSSpartanProof::<E>::prove(
            &conv.shape,
            &ck,
            u.u(),
            u.X(),
            &w,
            &mut transcript,
        )
        .map_err(|e| Error::Other(format!("Spartan prove: {e}")))?;

        Ok((
            SpartanProof {
                proof,
                instance: u,
            },
            SpartanVerifierKey {
                shape: conv.shape,
                pcs_vk,
                num_constraints: conv.num_constraints,
                num_constraints_padded: conv.num_constraints_padded,
                num_vars: conv.num_vars,
                num_io: conv.num_io,
            },
        ))
    }

    /// Verify a transparent decider proof against `vk`.
    pub fn verify(vk: &SpartanVerifierKey, proof: &SpartanProof) -> Result<bool, Error> {
        let mut transcript = <E as Engine>::TE::new(TRANSCRIPT_LABEL);
        proof.instance.absorb_commitments(&mut transcript);
        proof
            .proof
            .verify(&vk.shape, &vk.pcs_vk, &proof.instance, &mut transcript)
            .map_err(|_| Error::SNARKVerificationFail)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr;
    use ark_r1cs_std::fields::fp::FpVar;
    use ark_r1cs_std::prelude::*;
    use ark_relations::r1cs::SynthesisError;

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
    fn tiny_circuit_proves_transparent() {
        let circuit = TinyMul {
            a: Fr::from(3u64),
            b: Fr::from(5u64),
            c: Fr::from(15u64),
        };
        let (proof, vk) = SpartanDecider::prove(circuit).expect("prove");
        assert!(SpartanDecider::verify(&vk, &proof).expect("verify"));
        assert!(vk.num_constraints > 0);
    }
}
