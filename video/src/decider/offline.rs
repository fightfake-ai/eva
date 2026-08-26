//! Offline (non-EVM) wrap of a finished Nova IVC run.
//!
//! [`DeciderEthCircuit`] encodes CycleFold in **non-native** BN254 limbs so a
//! Solidity Groth16 verifier never implements Grumpkin (~7M constraints, OOM in
//! wasm). This module does **not** do that:
//!
//! 1. **Native IVC verify** — [`FoldingScheme::verify`] checks primary R1CS,
//!    CycleFold R1CS, and the Poseidon links, all on the CPU in each curve’s
//!    own field (see `folding-schemes/src/folding/nova/mod.rs` `Nova::verify`).
//! 2. **Native-field Spartan** — [`NativePrimaryCircuit`] re-checks only the
//!    **primary** relaxed R1CS with `FpVar` (`RelaxedR1CSGadget::check_native`).
//!    No `NonNativeUintVar`, no CycleFold gadgets, no in-circuit σ.
//! 3. **Camera σ** — verified in ordinary code by the caller, not in this SNARK.
//!
//! Documented in `docs/offline-decider-and-wasm-spartan.md`.

use ark_ff::PrimeField;
use ark_r1cs_std::{
    alloc::AllocVar,
    eq::EqGadget,
    fields::fp::FpVar,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_bn254::Fr;

use folding_schemes::ccs::r1cs::R1CS;
use folding_schemes::Error;

use super::{RelaxedR1CSGadget, R1CSVar};

#[cfg(feature = "spartan")]
use super::{SpartanDecider, SpartanProof, SpartanVerifierKey};

/// Ark circuit: `Az ∘ Bz = u Cz + E` over the **native** field of the Nova step
/// R1CS (BN254 `Fr` today). Matrices are constants; `z` is `[u | x | QW]`.
pub struct NativePrimaryCircuit<F: PrimeField> {
    pub r1cs: R1CS<F>,
    pub z: Vec<F>,
    pub u: F,
    pub e: Vec<F>,
}

impl<F: PrimeField> ConstraintSynthesizer<F> for NativePrimaryCircuit<F> {
    fn generate_constraints(self, cs: ConstraintSystemRef<F>) -> Result<(), SynthesisError> {
        let io_len = self.r1cs.l;
        let r1cs_var = R1CSVar::<F, F, FpVar<F>>::new_witness(cs.clone(), || Ok(self.r1cs))?;
        if self.z.is_empty() {
            return Err(SynthesisError::AssignmentMissing);
        }
        let u = FpVar::new_input(cs.clone(), || Ok(self.u))?;
        let mut z_vars = Vec::with_capacity(self.z.len());
        z_vars.push(u.clone());
        for (i, val) in self.z.iter().enumerate().skip(1) {
            let v = if i <= io_len {
                FpVar::new_input(cs.clone(), || Ok(*val))?
            } else {
                FpVar::new_witness(cs.clone(), || Ok(*val))?
            };
            z_vars.push(v);
        }
        let e_vars = self
            .e
            .iter()
            .map(|e| FpVar::new_witness(cs.clone(), || Ok(*e)))
            .collect::<Result<Vec<_>, _>>()?;
        RelaxedR1CSGadget::check_native(r1cs_var, e_vars, u, z_vars)
    }
}

/// Result of the offline wrap (IVC always; Spartan when the `spartan` feature is on).
#[derive(Clone, Debug)]
pub struct OfflineWrap {
    pub ivc_verified: bool,
    pub spartan_verified: bool,
    pub proof_bytes_len: usize,
    pub num_constraints: usize,
    pub proof_system: &'static str,
    pub wrap_error: Option<String>,
}

impl OfflineWrap {
    pub fn ok(&self) -> bool {
        self.ivc_verified && self.spartan_verified && self.wrap_error.is_none()
    }
}

/// Well-known id for the wrap circuit: gadget + blocks-per-step, not photo size.
pub fn native_primary_circuit_id(gadget: &str, bps: usize) -> String {
    format!("ff.native-primary.{gadget}.bps{bps}.v1")
}

/// Build the native primary circuit from a running Nova instance.
pub fn native_primary_from_running<F: PrimeField>(
    r1cs: R1CS<F>,
    u: F,
    x: Vec<F>,
    qw: Vec<F>,
    e: Vec<F>,
) -> NativePrimaryCircuit<F> {
    NativePrimaryCircuit {
        z: [vec![u], x, qw].concat(),
        u,
        e,
        r1cs,
    }
}

/// Dummy assignment with the right lengths so [`SpartanDecider::setup`] can
/// extract the wrap shape without a satisfying Nova instance.
pub fn dummy_native_primary<F: PrimeField>(r1cs: R1CS<F>) -> NativePrimaryCircuit<F> {
    let n = r1cs.A.n_cols;
    let m = r1cs.A.n_rows;
    NativePrimaryCircuit {
        z: vec![F::zero(); n.max(1)],
        u: F::zero(),
        e: vec![F::zero(); m],
        r1cs,
    }
}

#[cfg(feature = "spartan")]
pub fn prove_native_primary<F: PrimeField>(
    circuit: NativePrimaryCircuit<F>,
) -> Result<(SpartanProof, SpartanVerifierKey), Error> {
    SpartanDecider::prove(circuit)
}

#[cfg(feature = "spartan")]
pub fn setup_native_primary<F: PrimeField>(
    circuit: NativePrimaryCircuit<F>,
) -> Result<SpartanVerifierKey, Error> {
    SpartanDecider::setup(circuit)
}

#[cfg(feature = "spartan")]
pub fn verify_native_primary(vk: &SpartanVerifierKey, proof: &SpartanProof) -> Result<bool, Error> {
    SpartanDecider::verify(vk, proof)
}

/// Tiny `a * b = c` circuit used to smoke-test Spartan2 in wasm (not the edit wrap).
struct TinyMul {
    a: Fr,
    b: Fr,
    c: Fr,
}

impl ConstraintSynthesizer<Fr> for TinyMul {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Fr>,
    ) -> Result<(), SynthesisError> {
        let a = FpVar::new_witness(cs.clone(), || Ok(self.a))?;
        let b = FpVar::new_witness(cs.clone(), || Ok(self.b))?;
        let c = FpVar::new_input(cs, || Ok(self.c))?;
        let prod = &a * &b;
        prod.enforce_equal(&c)?;
        Ok(())
    }
}

/// Result of [`spartan_tiny_smoke`].
#[cfg(feature = "spartan")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct SpartanSmoke {
    pub verified: bool,
    pub num_constraints: usize,
    pub num_constraints_padded: usize,
    pub num_vars: usize,
}

/// Prove/verify a ~1-constraint native-field circuit with [`SpartanDecider`].
///
/// This answers “does Spartan2 run under wasm32?” without Nova or `EditOnlyCircuit`.
#[cfg(feature = "spartan")]
pub fn spartan_tiny_smoke() -> Result<SpartanSmoke, Error> {
    let (proof, vk) = SpartanDecider::prove(TinyMul {
        a: Fr::from(3u64),
        b: Fr::from(5u64),
        c: Fr::from(15u64),
    })?;
    Ok(SpartanSmoke {
        verified: SpartanDecider::verify(&vk, &proof)?,
        num_constraints: vk.num_constraints,
        num_constraints_padded: vk.num_constraints_padded,
        num_vars: vk.num_vars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;

    #[test]
    fn tiny_native_circuit_has_far_fewer_constraints_than_eth_decider() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        TinyMul {
            a: Fr::from(3u64),
            b: Fr::from(5u64),
            c: Fr::from(15u64),
        }
        .generate_constraints(cs.clone())
        .unwrap();
        cs.finalize();
        let n = cs.borrow().unwrap().num_constraints;
        assert!(n < 100, "tiny circuit should be tiny, got {n}");
        assert!(n < 7_000_000, "must stay well below DeciderEthCircuit");
    }

    #[cfg(feature = "spartan")]
    #[test]
    fn tiny_spartan_on_native_field_circuit() {
        let smoke = spartan_tiny_smoke().expect("prove");
        assert!(smoke.verified);
        assert!(smoke.num_constraints < 10_000);
    }
}
