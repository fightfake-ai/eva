//! Synthesize Eva's arkworks circuits and extract R1CS for analysis.
//!
//! Two circuit kinds are exported:
//!
//! - **Step-only** — `EditEncodeCircuit` with lookup constraints (the F circuit).
//! - **Augmented** — full Nova `AugmentedFCircuit` including IVC verifier gadgets and CycleFold hooks.
//!
//! Phase 1 will size bellpepper placeholders from the step-only stats; the augmented stats
//! represent what Nova actually folds per IVC step.

use std::marker::PhantomData;
use std::sync::Arc;

use ark_bn254::{Fr, G1Projective as Projective};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem, ConstraintSystemRef, SynthesisError};
use ark_ff::UniformRand;
use ark_std::rand::Rng;
use folding_schemes::ccs::r1cs::{extract_r1cs, R1CS};
use folding_schemes::folding::nova::circuits::AugmentedFCircuit;
use folding_schemes::frontend::{FCircuit, LookupArgument, LookupArgumentRef};
use folding_schemes::transcript::poseidon::poseidon_test_config;
use rand::thread_rng;
use video::edit::constraints::{EditGadget, NoOp};
use video::griffin::params::GriffinParams;
use video::{EditEncodeCircuit, ExternalInputs};

/// Which Eva circuit to synthesize for R1CS extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaCircuitKind {
    /// `EditEncodeCircuit` + lookup argument constraints only.
    StepOnly,
    /// Full Nova `AugmentedFCircuit` (F' in the Nova paper + CycleFold verifier).
    Augmented,
}

/// Read `BLOCKS_PER_STEP` from the environment, defaulting to `256`.
pub fn blocks_per_step_from_env() -> usize {
    std::env::var("BLOCKS_PER_STEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256)
}

/// Build default external inputs for synthesis (empty macroblock slots).
pub fn default_external_inputs(blocks_per_step: usize) -> ExternalInputs<()> {
    ExternalInputs {
        blocks: vec![Default::default(); blocks_per_step],
        predictions: vec![Default::default(); blocks_per_step],
        outputs: vec![Default::default(); blocks_per_step],
        encode_configs: vec![Default::default(); blocks_per_step],
        edit_configs: vec![(); blocks_per_step],
    }
}

/// Wrapper that synthesizes only the Eva F circuit (no Nova IVC verifier gadgets).
struct StepOnlyCircuit<'a, F: ark_ff::PrimeField + ark_crypto_primitives::sponge::Absorb, E: EditGadget> {
    f_circuit: &'a EditEncodeCircuit<F, E>,
    external: &'a ExternalInputs<E::Cfg>,
    la: LookupArgumentRef<F>,
}

impl<'a, F: ark_ff::PrimeField + ark_crypto_primitives::sponge::Absorb, E: EditGadget> ConstraintSynthesizer<F>
    for StepOnlyCircuit<'a, F, E>
{
    fn generate_constraints(self, cs: ConstraintSystemRef<F>) -> Result<(), SynthesisError> {
        let z_i: Vec<FpVar<F>> = (0..self.f_circuit.state_len())
            .map(|_| FpVar::new_input(cs.clone(), || Ok(F::from(0u64))))
            .collect::<Result<_, _>>()?;

        self.f_circuit.generate_step_constraints(
            cs,
            self.la,
            0,
            z_i,
            self.external,
        )?;
        Ok(())
    }
}

/// Synthesize the Eva **step-only** circuit (F + lookups) and return its R1CS.
pub fn synthesize_step_only_r1cs(blocks_per_step: usize) -> Result<R1CS<Fr>, String> {
    synthesize_step_only_r1cs_with_rng(blocks_per_step, &mut thread_rng())
}

/// Synthesize step-only R1CS with a provided RNG (needed for lookup randomness).
pub fn synthesize_step_only_r1cs_with_rng(
    blocks_per_step: usize,
    rng: &mut impl Rng,
) -> Result<R1CS<Fr>, String> {
    let la = LookupArgument::new_ref();
    let f_circuit = EditEncodeCircuit::<Fr, NoOp> {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };
    let external = default_external_inputs(blocks_per_step);

    let cs = ConstraintSystem::<Fr>::new_ref();
    StepOnlyCircuit {
        f_circuit: &f_circuit,
        external: &external,
        la: la.clone(),
    }
    .generate_constraints(cs.clone())
    .map_err(|e| format!("step constraint synthesis failed: {e}"))?;

    la.build_histo(cs.clone())
        .map_err(|e| format!("lookup histo failed: {e}"))?;
    la.generate_lookup_constraints(cs.clone(), Fr::rand(rng))
        .map_err(|e| format!("lookup constraints failed: {e}"))?;

    cs.finalize();
    let cs = cs.into_inner().ok_or("missing inner constraint system")?;
    Ok(extract_r1cs(&cs))
}

/// Synthesize the full Nova **augmented** step circuit and return its R1CS.
pub fn synthesize_augmented_step_r1cs(blocks_per_step: usize) -> Result<R1CS<Fr>, String> {
    synthesize_augmented_step_r1cs_with_rng(blocks_per_step, &mut thread_rng())
}

/// Synthesize augmented R1CS with a provided RNG.
pub fn synthesize_augmented_step_r1cs_with_rng(
    blocks_per_step: usize,
    rng: &mut impl Rng,
) -> Result<R1CS<Fr>, String> {
    let la = LookupArgument::new_ref();
    let f_circuit = EditEncodeCircuit::<Fr, NoOp> {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };
    let external = default_external_inputs(blocks_per_step);
    let poseidon_config = poseidon_test_config();

    let cs = ConstraintSystem::<Fr>::new_ref();
    AugmentedFCircuit::<Projective, Projective2, GVar2, _>::empty(
        &poseidon_config,
        la.clone(),
        &f_circuit,
        &external,
    )
    .run(cs.clone())
    .map_err(|e| format!("augmented circuit run failed: {e}"))?;

    la.build_histo(cs.clone())
        .map_err(|e| format!("lookup histo failed: {e}"))?;
    la.generate_lookup_constraints(cs.clone(), Fr::rand(rng))
        .map_err(|e| format!("lookup constraints failed: {e}"))?;

    cs.finalize();
    let cs = cs.into_inner().ok_or("missing inner constraint system")?;
    Ok(extract_r1cs(&cs))
}

/// Synthesize both circuit kinds and return `(step_only, augmented)` R1CS pair.
pub fn synthesize_both_r1cs(blocks_per_step: usize) -> Result<(R1CS<Fr>, R1CS<Fr>), String> {
    let mut rng = thread_rng();
    let step = synthesize_step_only_r1cs_with_rng(blocks_per_step, &mut rng)?;
    let augmented = synthesize_augmented_step_r1cs_with_rng(blocks_per_step, &mut rng)?;
    Ok((step, augmented))
}

/// Synthesize R1CS for the given [`EvaCircuitKind`].
pub fn synthesize_r1cs(kind: EvaCircuitKind, blocks_per_step: usize) -> Result<R1CS<Fr>, String> {
    match kind {
        EvaCircuitKind::StepOnly => synthesize_step_only_r1cs(blocks_per_step),
        EvaCircuitKind::Augmented => synthesize_augmented_step_r1cs(blocks_per_step),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folding_schemes::folding::nova::get_r1cs_from_cs;

    /// Sanity check: `get_r1cs_from_cs` path matches direct extraction (without lookups).
    #[test]
    fn step_only_wrapper_synthesizes() {
        let la = LookupArgument::new_ref();
        let f_circuit = EditEncodeCircuit::<Fr, NoOp> {
            _e: PhantomData,
            griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
        };
        let external = default_external_inputs(4);
        let r1cs = get_r1cs_from_cs(StepOnlyCircuit {
            f_circuit: &f_circuit,
            external: &external,
            la,
        })
        .expect("step-only synthesis");
        assert!(r1cs.A.n_rows > 0);
    }
}
