//! Spartan2 NeutronNova reference benchmark on BN254.
//!
//! Phase 0 uses a **placeholder** bellpepper circuit (repeated squaring chain) — not Eva's
//! actual logic. The purpose is to validate the Spartan2 toolchain and establish timing
//! infrastructure before Phase 1 sizes the circuit to match Eva's R1CS.
//!
//! # Circuit design notes
//!
//! Spartan2's NeutronNova API requires:
//! - A **step circuit** prototype for `setup`
//! - A **core circuit** prototype for `setup` (can be the same type)
//! - `num_steps` step circuit instances + one core instance at prove time
//!
//! The placeholder follows Spartan2's SHA-256 unit-test pattern: constraints live in
//! `synthesize` (not `precommitted`), a single public input is exposed via `inputize`,
//! and `public_values()` returns `[ZERO]`.
//!
//! **Note:** On BN254, placing constraints only in `precommitted` caused verify failures
//! during Phase 0 bring-up. Phase 1 will revisit the `prep_prove` split once we port
//! Eva's precomputable witness (video blocks) vs online witness (challenges).

use std::marker::PhantomData;
use std::time::Instant;

use bellpepper_core::{num::AllocatedNum, ConstraintSystem, SynthesisError};
use ff::Field;
use spartan2::{
    neutronnova_zk::NeutronNovaZkSNARK,
    provider::Bn254Engine,
    traits::{Engine, circuit::SpartanCircuit},
};

type E = Bn254Engine;

/// Configuration for the Phase 0 placeholder NeutronNova benchmark.
#[derive(Clone, Debug)]
pub struct PlaceholderConfig {
    /// Number of step circuits in the batch (padded to next power of two internally by Spartan2).
    pub num_steps: usize,
    /// Number of squaring constraints per step circuit (controls R1CS size).
    pub circuit_degree: usize,
    /// Pass `is_small=true` to Spartan2 for bit-width-aware fast paths.
    /// Only valid when witness values remain small — not suitable for squaring chains.
    pub is_small: bool,
}

impl PlaceholderConfig {
    /// Load from environment:
    /// - `NUM_STEPS` (default 4)
    /// - `CIRCUIT_DEGREE` (default 1024)
    /// - `IS_SMALL` (default false — squaring overflows small-integer fast path)
    pub fn from_env() -> Self {
        Self {
            num_steps: std::env::var("NUM_STEPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4),
            circuit_degree: std::env::var("CIRCUIT_DEGREE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1024),
            is_small: std::env::var("IS_SMALL")
                .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }
}

/// Timing breakdown for one NeutronNova prove pipeline run.
#[derive(Clone, Debug, Default)]
pub struct NeutronNovaTimings {
    pub setup_ms: f64,
    pub prep_prove_ms: f64,
    pub prove_ms: f64,
    pub verify_ms: f64,
    pub num_steps: usize,
    pub circuit_degree: usize,
    pub is_small: bool,
}

impl std::fmt::Display for NeutronNovaTimings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NeutronNova(BN254): steps={} degree={} is_small={} setup={:.1}ms prep={:.1}ms prove={:.1}ms verify={:.1}ms",
            self.num_steps,
            self.circuit_degree,
            self.is_small,
            self.setup_ms,
            self.prep_prove_ms,
            self.prove_ms,
            self.verify_ms,
        )
    }
}

/// Placeholder step/core circuit: chain of squaring constraints `x_{i+1} = x_i * x_i`.
///
/// Phase 1 will replace this with a size-matched circuit based on [`super::r1cs_stats::R1csStats`].
#[derive(Clone, Debug)]
pub struct PlaceholderStepCircuit {
    pub degree: usize,
    /// Starting value for the squaring chain (must be provided in witness).
    pub seed: u64,
    _p: PhantomData<E>,
}

impl PlaceholderStepCircuit {
    pub fn new(degree: usize, seed: u64) -> Self {
        Self {
            degree,
            seed,
            _p: PhantomData,
        }
    }
}

impl SpartanCircuit<E> for PlaceholderStepCircuit {
    fn public_values(&self) -> Result<Vec<<E as Engine>::Scalar>, SynthesisError> {
        // Spartan2 NeutronNova examples expose a single public IO via inputize; the declared
        // public value is zero in the reference SHA-256 benchmark.
        Ok(vec![<E as Engine>::Scalar::ZERO])
    }

    fn shared<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        _cs: &mut CS,
    ) -> Result<Vec<AllocatedNum<<E as Engine>::Scalar>>, SynthesisError> {
        Ok(vec![])
    }

    fn precommitted<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        _cs: &mut CS,
        _shared: &[AllocatedNum<<E as Engine>::Scalar>],
    ) -> Result<Vec<AllocatedNum<<E as Engine>::Scalar>>, SynthesisError> {
        Ok(vec![])
    }

    fn num_challenges(&self) -> usize {
        0
    }

    fn synthesize<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        cs: &mut CS,
        _shared: &[AllocatedNum<<E as Engine>::Scalar>],
        _precommitted: &[AllocatedNum<<E as Engine>::Scalar>],
        _challenges: Option<&[<E as Engine>::Scalar]>,
    ) -> Result<(), SynthesisError> {
        let mut chain = vec![<E as Engine>::Scalar::from(self.seed)];
        for _ in 0..self.degree {
            chain.push(chain.last().unwrap().square());
        }

        let mut x = AllocatedNum::alloc(cs.namespace(|| "x0"), || Ok(chain[0]))?;

        for i in 0..self.degree {
            let y = AllocatedNum::alloc(cs.namespace(|| format!("y_{i}")), || Ok(chain[i + 1]))?;
            cs.enforce(
                || format!("square {i}"),
                |lc| lc + x.get_variable(),
                |lc| lc + x.get_variable(),
                |lc| lc + y.get_variable(),
            );
            x = y;
        }

        x.inputize(cs.namespace(|| "inputize x"))?;
        Ok(())
    }
}

/// Run full NeutronNova pipeline: setup → prep_prove → prove → verify.
pub fn run_neutronnova_smoke(config: &PlaceholderConfig) -> Result<NeutronNovaTimings, String> {
    let proto = PlaceholderStepCircuit::new(config.circuit_degree, 2);

    let t0 = Instant::now();
    let (pk, vk) = NeutronNovaZkSNARK::<E>::setup(&proto, &proto, config.num_steps)
        .map_err(|e| format!("NeutronNova setup failed: {e}"))?;
    let setup_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let step_circuits: Vec<PlaceholderStepCircuit> = (0..config.num_steps)
        .map(|i| PlaceholderStepCircuit::new(config.circuit_degree, i as u64 + 2))
        .collect();
    // Core circuit ties the batch; use first step instance (Spartan2 test pattern).
    let core_circuit = step_circuits[0].clone();

    let t1 = Instant::now();
    let prep = NeutronNovaZkSNARK::<E>::prep_prove(
        &pk,
        &step_circuits,
        &core_circuit,
        config.is_small,
    )
    .map_err(|e| format!("NeutronNova prep_prove failed: {e}"))?;
    let prep_prove_ms = t1.elapsed().as_secs_f64() * 1000.0;

    let t2 = Instant::now();
    let (proof, _prep) = NeutronNovaZkSNARK::<E>::prove(
        &pk,
        &step_circuits,
        &core_circuit,
        prep,
        config.is_small,
    )
    .map_err(|e| format!("NeutronNova prove failed: {e}"))?;
    let prove_ms = t2.elapsed().as_secs_f64() * 1000.0;

    let t3 = Instant::now();
    proof
        .verify(&vk, config.num_steps)
        .map_err(|e| format!("NeutronNova verify failed: {e}"))?;
    let verify_ms = t3.elapsed().as_secs_f64() * 1000.0;

    Ok(NeutronNovaTimings {
        setup_ms,
        prep_prove_ms,
        prove_ms,
        verify_ms,
        num_steps: config.num_steps,
        circuit_degree: config.circuit_degree,
        is_small: config.is_small,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bellpepper::gadgets::sha256::sha256;
    use bellpepper::gadgets::boolean::{AllocatedBit, Boolean};
    use bellpepper_core::ConstraintSystem;
    use spartan2::provider::T256HyraxEngine;

    /// Minimal SHA-256 circuit copied from Spartan2's `neutronnova_zk` unit tests.
    #[derive(Clone, Debug)]
    struct Sha256Step<E: Engine> {
        preimage: Vec<u8>,
        _p: PhantomData<E>,
    }

    impl<E: Engine> SpartanCircuit<E> for Sha256Step<E> {
        fn public_values(&self) -> Result<Vec<E::Scalar>, SynthesisError> {
            Ok(vec![E::Scalar::ZERO])
        }
        fn shared<CS: ConstraintSystem<E::Scalar>>(
            &self,
            _: &mut CS,
        ) -> Result<Vec<AllocatedNum<E::Scalar>>, SynthesisError> {
            Ok(vec![])
        }
        fn precommitted<CS: ConstraintSystem<E::Scalar>>(
            &self,
            _: &mut CS,
            _: &[AllocatedNum<E::Scalar>],
        ) -> Result<Vec<AllocatedNum<E::Scalar>>, SynthesisError> {
            Ok(vec![])
        }
        fn num_challenges(&self) -> usize {
            0
        }
        fn synthesize<CS: ConstraintSystem<E::Scalar>>(
            &self,
            cs: &mut CS,
            _: &[AllocatedNum<E::Scalar>],
            _: &[AllocatedNum<E::Scalar>],
            _: Option<&[E::Scalar]>,
        ) -> Result<(), SynthesisError> {
            let bit_values: Vec<_> = self
                .preimage
                .clone()
                .into_iter()
                .flat_map(|byte| (0..8).map(move |i| (byte >> i) & 1u8 == 1u8))
                .map(Some)
                .collect();
            let preimage_bits = bit_values
                .into_iter()
                .enumerate()
                .map(|(i, b)| AllocatedBit::alloc(cs.namespace(|| format!("bit {i}")), b))
                .map(|b| b.map(Boolean::from))
                .collect::<Result<Vec<_>, _>>()?;
            let _ = sha256(cs.namespace(|| "sha256"), &preimage_bits)?;
            let x = AllocatedNum::alloc(cs.namespace(|| "x"), || Ok(E::Scalar::ZERO))?;
            x.inputize(cs.namespace(|| "inputize x"))?;
            Ok(())
        }
    }

    #[test]
    fn neutronnova_sha256_reference_t256() {
        type E = T256HyraxEngine;
        let num = 2;
        let proto = Sha256Step::<E> {
            preimage: vec![0u8; 32],
            _p: PhantomData,
        };
        let (pk, vk) = NeutronNovaZkSNARK::<E>::setup(&proto, &proto, num).unwrap();
        let steps: Vec<_> = (0..num)
            .map(|i| Sha256Step::<E> {
                preimage: vec![i as u8; 32],
                _p: PhantomData,
            })
            .collect();
        let core = steps[0].clone();
        let prep = NeutronNovaZkSNARK::<E>::prep_prove(&pk, &steps, &core, true).unwrap();
        let (proof, _) = NeutronNovaZkSNARK::<E>::prove(&pk, &steps, &core, prep, true).unwrap();
        proof.verify(&vk, num).expect("reference sha256 verify");
    }

    #[test]
    fn neutronnova_smoke_small_degree() {
        let config = PlaceholderConfig {
            num_steps: 2,
            circuit_degree: 8,
            is_small: false,
        };
        run_neutronnova_smoke(&config).expect("smoke test should pass");
    }
}
