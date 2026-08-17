//! Bellpepper circuit: check a **relaxed R1CS** instance `Az ∘ Bz = u·Cz + E`.
//!
//! Used by Option C (Nova IVC + Spartan final SNARK) to prove the primary folded
//! instance without Groth16. CycleFold / device-sigma checks are **not** included.

use std::marker::PhantomData;

use bellpepper_core::{num::AllocatedNum, ConstraintSystem, LinearCombination, SynthesisError};
use ff::{Field, PrimeField};
use spartan2::traits::{circuit::SpartanCircuit, Engine};

/// One sparse matrix row: `(coeff, column_index)` pairs.
pub type SparseRow<Scalar> = Vec<(Scalar, usize)>;

/// Witness + public data for a relaxed R1CS check.
#[derive(Clone, Debug)]
pub struct RelaxedR1CSWitness<Scalar: PrimeField> {
    /// Full z = `[u] || x || QW`.
    pub z: Vec<Scalar>,
    /// Error vector E (length = number of constraints).
    pub e: Vec<Scalar>,
    /// Relaxation scalar u (also z[0]).
    pub u: Scalar,
}

/// Sparse A,B,C over the Spartan scalar field.
#[derive(Clone, Debug)]
pub struct SparseR1CS<Scalar: PrimeField> {
    pub a: Vec<SparseRow<Scalar>>,
    pub b: Vec<SparseRow<Scalar>>,
    pub c: Vec<SparseRow<Scalar>>,
    pub num_vars: usize,
}

impl<Scalar: PrimeField> SparseR1CS<Scalar> {
    pub fn num_constraints(&self) -> usize {
        self.a.len()
    }
}

/// `SpartanCircuit` that re-encodes `Az∘Bz = uCz+E` as bellpepper constraints.
#[derive(Clone, Debug)]
pub struct RelaxedR1CSCheckCircuit<E: Engine> {
    pub r1cs: SparseR1CS<E::Scalar>,
    pub witness: Option<RelaxedR1CSWitness<E::Scalar>>,
    _p: PhantomData<E>,
}

impl<E: Engine> RelaxedR1CSCheckCircuit<E> {
    pub fn new(r1cs: SparseR1CS<E::Scalar>, witness: Option<RelaxedR1CSWitness<E::Scalar>>) -> Self {
        if let Some(w) = &witness {
            assert_eq!(w.z.len(), r1cs.num_vars);
            assert_eq!(w.e.len(), r1cs.num_constraints());
            assert_eq!(w.z[0], w.u);
        }
        Self {
            r1cs,
            witness,
            _p: PhantomData,
        }
    }
}

fn eval_row<Scalar: PrimeField>(row: &SparseRow<Scalar>, z: &[Scalar]) -> Scalar {
    let mut acc = Scalar::ZERO;
    for &(coeff, col) in row {
        acc += coeff * z[col];
    }
    acc
}

fn lc_row<Scalar: PrimeField>(
    row: &SparseRow<Scalar>,
    z: &[AllocatedNum<Scalar>],
    mut lc: LinearCombination<Scalar>,
) -> LinearCombination<Scalar> {
    for &(coeff, col) in row {
        lc = lc + (coeff, z[col].get_variable());
    }
    lc
}

impl<E: Engine> SpartanCircuit<E> for RelaxedR1CSCheckCircuit<E> {
    fn public_values(&self) -> Result<Vec<E::Scalar>, SynthesisError> {
        // Expose u as the sole public IO (plus Spartan2's required public slot).
        let u = self
            .witness
            .as_ref()
            .map(|w| w.u)
            .unwrap_or(E::Scalar::ZERO);
        Ok(vec![u])
    }

    fn shared<CS: ConstraintSystem<E::Scalar>>(
        &self,
        _cs: &mut CS,
    ) -> Result<Vec<AllocatedNum<E::Scalar>>, SynthesisError> {
        Ok(vec![])
    }

    fn precommitted<CS: ConstraintSystem<E::Scalar>>(
        &self,
        _cs: &mut CS,
        _shared: &[AllocatedNum<E::Scalar>],
    ) -> Result<Vec<AllocatedNum<E::Scalar>>, SynthesisError> {
        Ok(vec![])
    }

    fn num_challenges(&self) -> usize {
        0
    }

    fn synthesize<CS: ConstraintSystem<E::Scalar>>(
        &self,
        cs: &mut CS,
        _shared: &[AllocatedNum<E::Scalar>],
        _precommitted: &[AllocatedNum<E::Scalar>],
        _challenges: Option<&[E::Scalar]>,
    ) -> Result<(), SynthesisError> {
        let n_vars = self.r1cs.num_vars;
        let n_cons = self.r1cs.num_constraints();

        let z_vals = self.witness.as_ref().map(|w| w.z.clone());
        let e_vals = self.witness.as_ref().map(|w| w.e.clone());
        let u_val = self.witness.as_ref().map(|w| w.u);

        let mut z = Vec::with_capacity(n_vars);
        for i in 0..n_vars {
            let v = AllocatedNum::alloc(cs.namespace(|| format!("z_{i}")), || {
                Ok(z_vals
                    .as_ref()
                    .map(|z| z[i])
                    .unwrap_or(E::Scalar::ZERO))
            })?;
            z.push(v);
        }

        // Publicize u = z[0]
        z[0].inputize(cs.namespace(|| "inputize u"))?;

        for i in 0..n_cons {
            let az = AllocatedNum::alloc(cs.namespace(|| format!("az_{i}")), || {
                Ok(z_vals
                    .as_ref()
                    .map(|z| eval_row(&self.r1cs.a[i], z))
                    .unwrap_or(E::Scalar::ZERO))
            })?;
            cs.enforce(
                || format!("az_eq_{i}"),
                |lc| lc_row(&self.r1cs.a[i], &z, lc),
                |lc| lc + CS::one(),
                |lc| lc + az.get_variable(),
            );

            let bz = AllocatedNum::alloc(cs.namespace(|| format!("bz_{i}")), || {
                Ok(z_vals
                    .as_ref()
                    .map(|z| eval_row(&self.r1cs.b[i], z))
                    .unwrap_or(E::Scalar::ZERO))
            })?;
            cs.enforce(
                || format!("bz_eq_{i}"),
                |lc| lc_row(&self.r1cs.b[i], &z, lc),
                |lc| lc + CS::one(),
                |lc| lc + bz.get_variable(),
            );

            let cz = AllocatedNum::alloc(cs.namespace(|| format!("cz_{i}")), || {
                Ok(z_vals
                    .as_ref()
                    .map(|z| eval_row(&self.r1cs.c[i], z))
                    .unwrap_or(E::Scalar::ZERO))
            })?;
            cs.enforce(
                || format!("cz_eq_{i}"),
                |lc| lc_row(&self.r1cs.c[i], &z, lc),
                |lc| lc + CS::one(),
                |lc| lc + cz.get_variable(),
            );

            let e = AllocatedNum::alloc(cs.namespace(|| format!("e_{i}")), || {
                Ok(e_vals
                    .as_ref()
                    .map(|e| e[i])
                    .unwrap_or(E::Scalar::ZERO))
            })?;

            let ucz = AllocatedNum::alloc(cs.namespace(|| format!("ucz_{i}")), || {
                Ok(match (&z_vals, u_val) {
                    (Some(z), Some(u)) => u * eval_row(&self.r1cs.c[i], z),
                    _ => E::Scalar::ZERO,
                })
            })?;
            cs.enforce(
                || format!("u_cz_{i}"),
                |lc| lc + z[0].get_variable(),
                |lc| lc + cz.get_variable(),
                |lc| lc + ucz.get_variable(),
            );

            // az * bz = ucz + e
            cs.enforce(
                || format!("relaxed_{i}"),
                |lc| lc + az.get_variable(),
                |lc| lc + bz.get_variable(),
                |lc| lc + ucz.get_variable() + e.get_variable(),
            );
        }

        Ok(())
    }
}
