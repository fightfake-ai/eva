//! Nova (Eva) baseline timings for comparison against NeutronNova.
//!
//! Times the main cost centers of one Eva IVC step on the CPU backend:
//! - Augmented circuit R1CS synthesis
//! - `Nova::preprocess` (includes synthesis + Pedersen setup)
//! - `NIFS::compute_cmT` (compute_t + Pedersen MSM on T)
//! - Full `Nova::prove_step` for step i=0 (base case, no CycleFold fold yet)

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use ark_bn254::{constraints::GVar, Fr, G1Projective as Projective};
use ark_ff::Zero;
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use ark_std::rand::Rng;
use folding_schemes::commitment::pedersen::Pedersen;
use folding_schemes::folding::nova::nifs::NIFS;
use folding_schemes::folding::nova::traits::NovaR1CS;
use folding_schemes::folding::nova::Nova;
use folding_schemes::transcript::poseidon::poseidon_test_config;
use folding_schemes::{FoldingScheme, MVM};
use rand::thread_rng;
use video::edit::constraints::NoOp;
use video::griffin::params::GriffinParams;
use video::{EditEncodeCircuit};

use crate::eva_step::{default_external_inputs, synthesize_augmented_step_r1cs};
use crate::r1cs_stats::stats_from_r1cs;

/// Configuration for Nova baseline benchmarks.
#[derive(Clone, Debug)]
pub struct NovaBaselineConfig {
    pub blocks_per_step: usize,
}

impl NovaBaselineConfig {
    pub fn from_env() -> Self {
        Self {
            blocks_per_step: std::env::var("BLOCKS_PER_STEP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(256),
        }
    }
}

/// Timing breakdown for Nova baseline operations.
#[derive(Clone, Debug, Default)]
pub struct NovaBaselineTimings {
    pub blocks_per_step: usize,
    pub num_constraints: usize,
    pub synthesis_ms: f64,
    pub preprocess_ms: f64,
    pub compute_cmT_ms: f64,
    pub prove_step_ms: f64,
}

impl std::fmt::Display for NovaBaselineTimings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Nova(CPU): blocks={} constraints={} synthesis={:.1}ms preprocess={:.1}ms compute_cmT={:.1}ms prove_step={:.1}ms",
            self.blocks_per_step,
            self.num_constraints,
            self.synthesis_ms,
            self.preprocess_ms,
            self.compute_cmT_ms,
            self.prove_step_ms,
        )
    }
}

type EvaNova = Nova<
    Projective,
    GVar,
    Projective2,
    GVar2,
    EditEncodeCircuit<Fr, NoOp>,
    Pedersen<Projective>,
    Pedersen<Projective2>,
>;

fn eva_circuit() -> EditEncodeCircuit<Fr, NoOp> {
    EditEncodeCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    }
}

/// Run Nova baseline timings for the given configuration.
pub fn run_nova_baseline(config: &NovaBaselineConfig) -> Result<NovaBaselineTimings, String> {
    let mut rng = thread_rng();
    let f_circuit = eva_circuit();
    let external = default_external_inputs(config.blocks_per_step);
    let poseidon_config = poseidon_test_config();

    // --- R1CS synthesis (augmented circuit) ---
    let t0 = Instant::now();
    let r1cs = synthesize_augmented_step_r1cs(config.blocks_per_step)?;
    let stats = stats_from_r1cs("eva-augmented", &r1cs);
    let synthesis_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // --- preprocess (synthesis + Pedersen params) ---
    let t1 = Instant::now();
    let (pp, vp) = EvaNova::preprocess(&poseidon_config, &f_circuit, &mut rng, &external)
        .map_err(|e| format!("Nova preprocess failed: {e}"))?;
    let preprocess_ms = t1.elapsed().as_secs_f64() * 1000.0;

    // --- compute_cmT hot path (compute_t + commit to T) ---
    let (w1, ci1) = vp.r1cs.dummy_running_instance();
    let (w2, ci2) = vp.r1cs.dummy_current_instance();
    let e = Fr::alloc_vec(vp.r1cs.A.n_rows);
    let mut t = Fr::alloc_vec(vp.r1cs.A.n_rows);

    let t2 = Instant::now();
    let _cm_t = NIFS::<Projective, Pedersen<Projective, false>>::compute_cmT(
        None,
        &pp.cs_params,
        &vp.r1cs,
        &w1,
        &ci1,
        &w2,
        &ci2,
        &e,
        &mut t,
    )
    .map_err(|e| format!("compute_cmT failed: {e}"))?;
    let compute_cmT_ms = t2.elapsed().as_secs_f64() * 1000.0;

    // --- full prove_step (i=0 base case) ---
    let t3 = Instant::now();
    let params = (pp.clone(), vp.clone());
    let mut nova = EvaNova::init(&params, f_circuit.clone(), vec![Fr::zero(), Fr::zero()])
        .map_err(|e| format!("Nova init failed: {e}"))?;
    nova.prove_step(&params, &external)
        .map_err(|e| format!("Nova prove_step failed: {e}"))?;
    let prove_step_ms = t3.elapsed().as_secs_f64() * 1000.0;

    Ok(NovaBaselineTimings {
        blocks_per_step: config.blocks_per_step,
        num_constraints: stats.num_constraints,
        synthesis_ms,
        preprocess_ms,
        compute_cmT_ms,
        prove_step_ms,
    })
}

/// Quick mode config for fast iteration (`BLOCKS_PER_STEP=4`).
pub fn quick_nova_config() -> NovaBaselineConfig {
    NovaBaselineConfig {
        blocks_per_step: 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nova_baseline_quick() {
        let timings = run_nova_baseline(&quick_nova_config()).expect("nova baseline");
        assert!(timings.num_constraints > 0);
        assert!(timings.prove_step_ms >= 0.0);
        eprintln!("{timings}");
    }
}
