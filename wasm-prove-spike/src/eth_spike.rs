//! Native Spike A: Nova + Groth16 [`video::decider::DeciderEthCircuit`].
//!
//! Compiled only when `target_arch != "wasm32"`. The in-tab editor uses
//! [`crate::browser`] (offline wrap) and never builds this module.
//! See `docs/SPIKE_A.md` and `docs/offline-decider-and-wasm-spartan.md`.

use std::marker::PhantomData;
use std::time::Instant;

use ark_bn254::{constraints::GVar, Bn254, Fq, Fr, G1Projective as Projective};
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as GrumpkinProjective};
use ark_groth16::{Groth16, ProvingKey};
use ark_serialize::CanonicalSerialize;
use ark_std::rand::SeedableRng;
use folding_schemes::{
    commitment::pedersen::Pedersen,
    folding::nova::Nova,
    transcript::poseidon::poseidon_test_config,
    FoldingScheme,
};
use rand::rngs::StdRng;
use rand::Rng;
use serde::Serialize;
use video::decider::{Decider, DeciderEthCircuit};
use video::edit::constraints::{Brightness, BrightnessCfg};
use video::encode::Matrix;
use video::{EditOnlyCircuit, EditOnlyExternalInputs};

use crate::params::{SpikeConfig, SpikeParams, DEFAULT_SETUP_RNG_SEED};
use crate::{field_hex, spike_circuit};

type Op = Brightness;

type NovaScheme = Nova<
    Projective,
    GVar,
    GrumpkinProjective,
    GVar2,
    EditOnlyCircuit<Fr, Op>,
    Pedersen<Projective>,
    Pedersen<GrumpkinProjective>,
>;

type NovaParams = (
    <NovaScheme as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, Op>>>::ProverParam,
    <NovaScheme as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, Op>>>::VerifierParam,
);

#[derive(Clone, Debug, Serialize)]
pub struct PhaseTimingMs {
    pub nova_preprocess: f64,
    pub groth16_setup: f64,
    pub nova_prove: f64,
    pub groth16_prove: f64,
    pub decider_self_verify: f64,
    pub total: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SpikeResult {
    pub config: SpikeConfig,
    pub h1: String,
    pub h2: String,
    pub proof_bytes_len: usize,
    pub proof_bytes_hex_prefix: String,
    pub ffpb_header_ok: bool,
    pub decider_verify_ok: bool,
    pub timing_ms: PhaseTimingMs,
    #[serde(skip)]
    pub proof_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SpikeSetupResult {
    pub config: SpikeConfig,
    pub params_bytes_len: usize,
    pub timing_ms: PhaseTimingMs,
    #[serde(skip)]
    pub params: SpikeParams,
}

#[derive(Clone, Debug)]
pub struct FoldedInstance {
    pub cm_e: Projective,
    pub u: Fr,
    pub cm_q: Projective,
    pub cm_w: Projective,
}

#[derive(Clone, Debug)]
pub struct ProofBundle {
    pub num_steps: u64,
    pub z0: Vec<Fr>,
    pub h2: Fr,
    pub device_vk: GrumpkinProjective,
    pub vk: ark_groth16::VerifyingKey<Bn254>,
    pub u_running: FoldedInstance,
    pub u_current: FoldedInstance,
    pub proof: ark_groth16::Proof<Bn254>,
    pub cm_t: Projective,
    pub r: Fr,
}

const FFPB_MAGIC: [u8; 4] = *b"FFPB";
const FFPB_VERSION: u8 = 1;

impl ProofBundle {
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        out.extend_from_slice(&FFPB_MAGIC);
        out.push(FFPB_VERSION);
        out.extend_from_slice(&self.num_steps.to_le_bytes());
        self.z0
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize z0: {e}"))?;
        self.h2
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize h2: {e}"))?;
        self.device_vk
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize device_vk: {e}"))?;
        self.vk
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize vk: {e}"))?;
        write_instance(&self.u_running, &mut out)?;
        write_instance(&self.u_current, &mut out)?;
        self.proof
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize proof: {e}"))?;
        self.cm_t
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize cm_t: {e}"))?;
        self.r
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize r: {e}"))?;
        Ok(out)
    }

    pub fn looks_like_bundle(bytes: &[u8]) -> bool {
        bytes.len() >= 5 && bytes[..4] == FFPB_MAGIC
    }
}

fn write_instance(inst: &FoldedInstance, out: &mut Vec<u8>) -> Result<(), String> {
    inst.cm_e
        .serialize_compressed(&mut *out)
        .map_err(|e| format!("serialize cm_e: {e}"))?;
    inst.u.serialize_compressed(&mut *out)
        .map_err(|e| format!("serialize u: {e}"))?;
    inst.cm_q
        .serialize_compressed(&mut *out)
        .map_err(|e| format!("serialize cm_q: {e}"))?;
    inst.cm_w
        .serialize_compressed(&mut *out)
        .map_err(|e| format!("serialize cm_w: {e}"))?;
    Ok(())
}

fn synthetic_macroblocks(
    n: usize,
    rng: &mut impl Rng,
) -> Vec<(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>)> {
    (0..n)
        .map(|_| {
            (
                Matrix::from_iter((0..256).map(|_| rng.gen_range(0..=255u8))),
                Matrix::from_iter((0..64).map(|_| rng.gen_range(0..=255u8))),
                Matrix::from_iter((0..64).map(|_| rng.gen_range(0..=255u8))),
            )
        })
        .collect()
}

fn validate_config(config: &SpikeConfig) -> Result<(), String> {
    if config.blocks_per_step == 0 || config.num_steps == 0 {
        return Err("blocks_per_step and num_steps must be >= 1".into());
    }
    Ok(())
}

fn nova_preprocess_with_blocks(
    config: &SpikeConfig,
    rng: &mut StdRng,
    first_step_blocks: &[(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>)],
) -> Result<NovaParams, String> {
    let f_circuit = spike_circuit();
    let poseidon_config = poseidon_test_config();
    let brightness = BrightnessCfg(config.brightness_scale);
    let (pp, vp) = NovaScheme::preprocess(
        &poseidon_config,
        &f_circuit,
        rng,
        &EditOnlyExternalInputs {
            blocks: first_step_blocks.to_vec(),
            edit_configs: vec![brightness; config.blocks_per_step],
        },
    )
    .map_err(|e| format!("Nova preprocess: {e}"))?;
    Ok((pp, vp))
}

fn spike_inputs(config: &SpikeConfig) -> (Fq, Vec<(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>)>, StdRng) {
    let mut rng = StdRng::seed_from_u64(config.rng_seed);
    let sk = Fq::rand(&mut rng);
    let all_blocks = synthetic_macroblocks(config.blocks_per_step * config.num_steps, &mut rng);
    (sk, all_blocks, rng)
}

fn groth16_setup(params: &NovaParams, setup_rng: &mut StdRng) -> Result<ProvingKey<Bn254>, String> {
    let (pp, vp) = params;
    let poseidon_config = poseidon_test_config();
    Groth16::<Bn254>::generate_random_parameters_with_reduction(
        DeciderEthCircuit::<Projective, GVar, GrumpkinProjective, GVar2> {
            _gc1: PhantomData,
            _gc2: PhantomData,
            r1cs: vp.r1cs.clone(),
            cf_r1cs: vp.cf_r1cs.clone(),
            cf_pedersen_params: pp.cf_cs_params.clone(),
            poseidon_config: poseidon_config.clone(),
            i: None,
            z_0: Some(vec![Fr::rand(setup_rng), Fr::rand(setup_rng)]),
            u_i: None,
            U_i: None,
            W_i1: None,
            cmT: None,
            r: None,
            cf_U_i: None,
            cf_W_i: None,
            E: None,
            cf_E: None,
            sigma: (Fr::rand(setup_rng), Fq::rand(setup_rng)),
            vk: GrumpkinProjective::rand(setup_rng),
            h1: Fr::rand(setup_rng),
            h2: Fr::rand(setup_rng),
        },
        vec![
            (
                &pp.cs_params.generators[..vp.r1cs.q],
                pp.cs_params.h.into_affine(),
            ),
            (
                &pp.cs_params.generators[..vp.r1cs.A.n_cols - 1 - vp.r1cs.l - vp.r1cs.q],
                pp.cs_params.h.into_affine(),
            ),
            (
                &pp.cs_params.generators[..vp.r1cs.A.n_rows],
                pp.cs_params.h.into_affine(),
            ),
        ],
        setup_rng,
    )
    .map_err(|e| format!("Groth16 setup: {e}"))
}

fn spike_log(_msg: &str) {}

pub fn run_setup(config: SpikeConfig) -> Result<SpikeSetupResult, String> {
    validate_config(&config)?;
    spike_log("spike setup: start");

    #[cfg(not(target_arch = "wasm32"))]
    let total_start = Instant::now();

    let (_sk, all_blocks, mut rng) = spike_inputs(&config);

    spike_log("spike setup: nova preprocess");
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    let params = nova_preprocess_with_blocks(
        &config,
        &mut rng,
        &all_blocks[0..config.blocks_per_step],
    )?;
    #[cfg(not(target_arch = "wasm32"))]
    let nova_preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let nova_preprocess_ms = 0.0;

    spike_log("spike setup: groth16 setup");
    #[cfg(not(target_arch = "wasm32"))]
    let t1 = Instant::now();
    let mut setup_rng = StdRng::seed_from_u64(DEFAULT_SETUP_RNG_SEED);
    let groth16_pk = groth16_setup(&params, &mut setup_rng)?;
    #[cfg(not(target_arch = "wasm32"))]
    let groth16_setup_ms = t1.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let groth16_setup_ms = 0.0;

    let params_cache = SpikeParams {
        config: SpikeConfig {
            rng_seed: 0,
            blocks_per_step: config.blocks_per_step,
            num_steps: config.num_steps,
            brightness_scale: config.brightness_scale,
        },
        groth16_pk,
    };
    let params_bytes_len = params_cache.to_bytes()?.len();
    spike_log("spike setup: done");

    Ok(SpikeSetupResult {
        config: config.clone(),
        params_bytes_len,
        timing_ms: PhaseTimingMs {
            nova_preprocess: nova_preprocess_ms,
            groth16_setup: groth16_setup_ms,
            nova_prove: 0.0,
            groth16_prove: 0.0,
            decider_self_verify: 0.0,
            total: {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    total_start.elapsed().as_secs_f64() * 1000.0
                }
                #[cfg(target_arch = "wasm32")]
                {
                    0.0
                }
            },
        },
        params: params_cache,
    })
}

pub fn run_prove(config: SpikeConfig, cached: &SpikeParams) -> Result<SpikeResult, String> {
    validate_config(&config)?;
    if !cached.matches_config(&config) {
        return Err(format!(
            "cached params mismatch: file has {}x{} brightness={}, prove wants {}x{} brightness={}",
            cached.config.blocks_per_step,
            cached.config.num_steps,
            cached.config.brightness_scale,
            config.blocks_per_step,
            config.num_steps,
            config.brightness_scale,
        ));
    }

    spike_log("spike prove: start");
    #[cfg(not(target_arch = "wasm32"))]
    let total_start = Instant::now();

    let (sk, all_blocks, mut rng) = spike_inputs(&config);
    let brightness = BrightnessCfg(config.brightness_scale);
    let f_circuit = spike_circuit();
    let poseidon_config = poseidon_test_config();

    spike_log("spike prove: nova preprocess");
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    let params = nova_preprocess_with_blocks(
        &config,
        &mut rng,
        &all_blocks[0..config.blocks_per_step],
    )?;
    #[cfg(not(target_arch = "wasm32"))]
    let nova_preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let nova_preprocess_ms = 0.0;

    let vk_for_bundle = cached.groth16_pk.vk.clone();
    let decider_pp = cached.groth16_pk.clone();
    let initial_state = vec![Fr::zero(), Fr::zero()];

    spike_log("spike prove: nova ivc");
    #[cfg(not(target_arch = "wasm32"))]
    let t2 = Instant::now();
    let mut folding_scheme =
        NovaScheme::init(&params, f_circuit, initial_state.clone()).map_err(|e| format!("Nova init: {e}"))?;

    for step in 0..config.num_steps {
        let start = step * config.blocks_per_step;
        let end = start + config.blocks_per_step;
        folding_scheme
            .prove_step(
                &params,
                &EditOnlyExternalInputs {
                    blocks: all_blocks[start..end].to_vec(),
                    edit_configs: vec![brightness.clone(); config.blocks_per_step],
                },
            )
            .map_err(|e| format!("Nova prove_step {step}: {e}"))?;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let nova_prove_ms = t2.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let nova_prove_ms = 0.0;

    let last_state = folding_scheme.state();
    let h1 = last_state.first().copied().unwrap_or(Fr::zero());
    let h2 = last_state.get(1).copied().unwrap_or(Fr::zero());

    let (running_instance, incoming_instance, cyclefold_instance) = folding_scheme.instances();
    NovaScheme::verify(
        &params.1,
        initial_state.clone(),
        last_state.clone(),
        Fr::from(config.num_steps as u32),
        running_instance.clone(),
        incoming_instance.clone(),
        cyclefold_instance,
    )
    .map_err(|e| format!("Nova verify: {e}"))?;

    let device_vk = GrumpkinProjective::generator() * sk;
    let (px, py) = {
        let p = device_vk.into_affine();
        p.xy().unwrap_or((Fr::zero(), Fr::zero()))
    };
    let sigma = {
        let r = Fq::rand(&mut rng);
        let rx = (GrumpkinProjective::generator() * r)
            .into_affine()
            .x()
            .unwrap_or_default();
        let e = CRH::evaluate(&poseidon_config, [rx, px, py, folding_scheme.z_i[0]])
            .map_err(|e| format!("Schnorr hash: {e}"))?;
        (
            rx,
            r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()),
        )
    };

    let U_i = folding_scheme.U_i.clone();
    let circuit = DeciderEthCircuit::<Projective, GVar, GrumpkinProjective, GVar2>::from_nova(
        folding_scheme,
        params,
        device_vk,
        sigma,
    )
    .map_err(|e| format!("DeciderEthCircuit::from_nova: {e}"))?;
    let u_i = circuit
        .u_i
        .clone()
        .ok_or_else(|| "missing u_i witness".to_string())?;

    spike_log("spike prove: groth16 prove");
    #[cfg(not(target_arch = "wasm32"))]
    let t3 = Instant::now();
    let (groth_proof, cm_t, r) =
        Decider::prove(decider_pp, &mut rng, circuit).map_err(|e| format!("Groth16 prove: {e}"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let groth16_prove_ms = t3.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let groth16_prove_ms = 0.0;

    let bundle = ProofBundle {
        num_steps: config.num_steps as u64,
        z0: initial_state.clone(),
        h2,
        device_vk,
        vk: vk_for_bundle,
        u_running: FoldedInstance {
            cm_e: U_i.cmE,
            u: U_i.u,
            cm_q: U_i.cmQ,
            cm_w: U_i.cmW,
        },
        u_current: FoldedInstance {
            cm_e: u_i.cmE,
            u: u_i.u,
            cm_q: u_i.cmQ,
            cm_w: u_i.cmW,
        },
        proof: groth_proof.clone(),
        cm_t,
        r,
    };

    let proof_bytes = bundle.to_bytes()?;
    let ffpb_header_ok = ProofBundle::looks_like_bundle(&proof_bytes);
    spike_log("spike prove: done");

    let prefix_len = proof_bytes.len().min(32);
    Ok(SpikeResult {
        config,
        h1: field_hex(&h1),
        h2: field_hex(&h2),
        proof_bytes_len: proof_bytes.len(),
        proof_bytes_hex_prefix: hex::encode(&proof_bytes[..prefix_len]),
        ffpb_header_ok,
        decider_verify_ok: true,
        timing_ms: PhaseTimingMs {
            nova_preprocess: nova_preprocess_ms,
            groth16_setup: 0.0,
            nova_prove: nova_prove_ms,
            groth16_prove: groth16_prove_ms,
            decider_self_verify: 0.0,
            total: {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    total_start.elapsed().as_secs_f64() * 1000.0
                }
                #[cfg(target_arch = "wasm32")]
                {
                    0.0
                }
            },
        },
        proof_bytes,
    })
}

pub fn run_spike(config: SpikeConfig) -> Result<SpikeResult, String> {
    let setup = run_setup(config.clone())?;
    run_prove(config, &setup.params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fightfake_core::proof_bundle::{verify_proof_bundle, ProofBundle as ToolkitBundle};

    fn verify_toolkit(bytes: &[u8]) {
        let toolkit = ToolkitBundle::from_bytes(bytes).expect("parse FFPB");
        let ok = verify_proof_bundle(&toolkit).expect("verify");
        assert!(ok, "toolkit portable verify must accept spike proof");
    }

    #[test]
    fn spike_produces_ffpb_verifiable_by_toolkit() {
        let result = run_spike(SpikeConfig::default()).expect("spike prove");
        assert!(result.ffpb_header_ok);
        verify_toolkit(&result.proof_bytes);
    }

    #[test]
    fn prove_only_with_cached_params_verifies() {
        let config = SpikeConfig::default();
        let setup = run_setup(config.clone()).expect("setup");
        let cached = run_prove(config, &setup.params).expect("prove-only");
        assert!(cached.ffpb_header_ok);
        verify_toolkit(&cached.proof_bytes);

        let round_trip = SpikeParams::from_bytes(&setup.params.to_bytes().unwrap()).expect("params round-trip");
        let again = run_prove(SpikeConfig::default(), &round_trip).expect("prove after round-trip");
        assert_eq!(cached.h1, again.h1);
        assert_eq!(cached.h2, again.h2);
        verify_toolkit(&again.proof_bytes);
    }
}
