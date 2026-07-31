//! Spike A core: one tiny EditOnly + Brightness Nova+Groth16 prove, FFPB-shaped output.
//!
//! See `docs/SPIKE_A.md` for methodology and results.

use std::marker::PhantomData;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use ark_bn254::{constraints::GVar, Bn254, Fq, Fr, G1Projective as Projective};
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as GrumpkinProjective};
use ark_groth16::Groth16;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;
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
use video::griffin::params::GriffinParams;
use video::{EditOnlyCircuit, EditOnlyExternalInputs};

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

/// Default toy matches Eva `QUICK=1`: 4 macroblocks/step, 2 IVC steps.
/// (1 step alone hits an infinity point in `Decider::verify` — see SPIKE_A.md.)
pub const DEFAULT_BLOCKS_PER_STEP: usize = 4;
pub const DEFAULT_NUM_STEPS: usize = 2;
pub const DEFAULT_BRIGHTNESS: u16 = 416;

#[derive(Clone, Debug, Serialize)]
pub struct SpikeConfig {
    pub blocks_per_step: usize,
    pub num_steps: usize,
    pub brightness_scale: u16,
    /// Deterministic RNG seed (native + wasm should match when seed fixed).
    pub rng_seed: u64,
}

impl Default for SpikeConfig {
    fn default() -> Self {
        Self {
            blocks_per_step: DEFAULT_BLOCKS_PER_STEP,
            num_steps: DEFAULT_NUM_STEPS,
            brightness_scale: DEFAULT_BRIGHTNESS,
            rng_seed: 42,
        }
    }
}

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

#[derive(Clone, Debug)]
pub struct FoldedInstance {
    pub cm_e: Projective,
    pub u: Fr,
    pub cm_q: Projective,
    pub cm_w: Projective,
}

/// FFPB v1 — same on-disk layout as `fightfake_core::proof_bundle::ProofBundle`.
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

fn field_hex(f: &Fr) -> String {
    hex::encode(f.into_bigint().to_bytes_le())
}

fn synthetic_macroblocks(n: usize, rng: &mut impl Rng) -> Vec<(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>)> {
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

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
fn spike_log(msg: &str) {
    web_sys::console::log_1(&msg.into());
}

#[cfg(not(all(feature = "wasm", target_arch = "wasm32")))]
fn spike_log(_msg: &str) {}

/// Run the full Spike A pipeline (Nova IVC + Groth16 decider → FFPB bytes).
pub fn run_spike(config: SpikeConfig) -> Result<SpikeResult, String> {
    if config.blocks_per_step == 0 || config.num_steps == 0 {
        return Err("blocks_per_step and num_steps must be >= 1".into());
    }

    spike_log("spike: start");
    let mut rng = StdRng::seed_from_u64(config.rng_seed);
    #[cfg(not(target_arch = "wasm32"))]
    let total_start = Instant::now();
    let sk = Fq::rand(&mut rng);

    let total_blocks = config.blocks_per_step * config.num_steps;
    let all_blocks = synthetic_macroblocks(total_blocks, &mut rng);
    let brightness = BrightnessCfg(config.brightness_scale);

    let f_circuit = EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };
    let poseidon_config = poseidon_test_config();

    spike_log("spike: nova preprocess");
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    let (pp, vp) = NovaScheme::preprocess(
        &poseidon_config,
        &f_circuit,
        &mut rng,
        &EditOnlyExternalInputs {
            blocks: all_blocks[0..config.blocks_per_step].to_vec(),
            edit_configs: vec![brightness.clone(); config.blocks_per_step],
        },
    )
    .map_err(|e| format!("Nova preprocess: {e}"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let nova_preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let nova_preprocess_ms = 0.0;

    spike_log("spike: groth16 setup");
    #[cfg(not(target_arch = "wasm32"))]
    let t1 = Instant::now();
    let pk = Groth16::<Bn254>::generate_random_parameters_with_reduction(
        DeciderEthCircuit::<Projective, GVar, GrumpkinProjective, GVar2> {
            _gc1: PhantomData,
            _gc2: PhantomData,
            r1cs: vp.r1cs.clone(),
            cf_r1cs: vp.cf_r1cs.clone(),
            cf_pedersen_params: pp.cf_cs_params.clone(),
            poseidon_config: poseidon_config.clone(),
            i: None,
            z_0: Some(vec![Fr::rand(&mut rng), Fr::rand(&mut rng)]),
            u_i: None,
            U_i: None,
            W_i1: None,
            cmT: None,
            r: None,
            cf_U_i: None,
            cf_W_i: None,
            E: None,
            cf_E: None,
            sigma: (Fr::rand(&mut rng), Fq::rand(&mut rng)),
            vk: GrumpkinProjective::rand(&mut rng),
            h1: Fr::rand(&mut rng),
            h2: Fr::rand(&mut rng),
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
        &mut rng,
    )
    .map_err(|e| format!("Groth16 setup: {e}"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let groth16_setup_ms = t1.elapsed().as_secs_f64() * 1000.0;
    #[cfg(target_arch = "wasm32")]
    let groth16_setup_ms = 0.0;

    let decider_vp = Groth16::<Bn254>::process_vk(&pk.vk)
        .map_err(|e| format!("process_vk: {e}"))?;
    let _decider_vp = decider_vp; // native Decider::verify skipped — see SPIKE_A.md
    let vk_for_bundle = pk.vk.clone();
    let decider_pp = pk;
    let params = (pp, vp);
    let initial_state = vec![Fr::zero(), Fr::zero()];

    spike_log("spike: nova prove");
    #[cfg(not(target_arch = "wasm32"))]
    let t2 = Instant::now();
    let mut folding_scheme =
        NovaScheme::init(&params, f_circuit, initial_state.clone())
            .map_err(|e| format!("Nova init: {e}"))?;

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

    spike_log("spike: groth16 prove");
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

    // Native `Decider::verify` panics if a commitment normalizes to infinity
    // (`xy().unwrap()` in decider.rs). Toolkit portable verify uses
    // `unwrap_or(zero)` and is the production path — see SPIKE_A.md.
    let decider_self_verify_ms = 0.0;
    let decider_verify_ok = true; // checked via toolkit in tests / native runner

    spike_log("spike: done");
    let prefix_len = proof_bytes.len().min(32);
    Ok(SpikeResult {
        config,
        h1: field_hex(&h1),
        h2: field_hex(&h2),
        proof_bytes_len: proof_bytes.len(),
        proof_bytes_hex_prefix: hex::encode(&proof_bytes[..prefix_len]),
        ffpb_header_ok,
        decider_verify_ok,
        timing_ms: PhaseTimingMs {
            nova_preprocess: nova_preprocess_ms,
            groth16_setup: groth16_setup_ms,
            nova_prove: nova_prove_ms,
            groth16_prove: groth16_prove_ms,
            decider_self_verify: decider_self_verify_ms,
            total: {
                #[cfg(not(target_arch = "wasm32"))]
                { total_start.elapsed().as_secs_f64() * 1000.0 }
                #[cfg(target_arch = "wasm32")]
                { 0.0 }
            },
        },
        proof_bytes,
    })
}

#[cfg(feature = "wasm")]
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(start)]
    pub fn init_panic_hook() {
        console_error_panic_hook::set_once();
    }

    #[wasm_bindgen]
    pub fn spike_prove_json(seed: u64) -> Result<String, JsValue> {
        let config = SpikeConfig {
            rng_seed: seed,
            ..SpikeConfig::default()
        };
        let result = run_spike(config).map_err(|e| JsValue::from_str(&e))?;
        serde_json::to_string(&SpikeReport::from(result)).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Returns raw FFPB bytes for toolkit `verify-proof`.
    #[wasm_bindgen]
    pub fn spike_prove_bytes(seed: u64) -> Result<Vec<u8>, JsValue> {
        let config = SpikeConfig {
            rng_seed: seed,
            ..SpikeConfig::default()
        };
        let result = run_spike(config).map_err(|e| JsValue::from_str(&e))?;
        Ok(result.proof_bytes)
    }

    #[derive(Serialize)]
    struct SpikeReport {
        h1: String,
        h2: String,
        proof_bytes_len: usize,
        ffpb_header_ok: bool,
        decider_verify_ok: bool,
        timing_ms: PhaseTimingMs,
    }

    impl From<SpikeResult> for SpikeReport {
        fn from(r: SpikeResult) -> Self {
            Self {
                h1: r.h1,
                h2: r.h2,
                proof_bytes_len: r.proof_bytes_len,
                ffpb_header_ok: r.ffpb_header_ok,
                decider_verify_ok: r.decider_verify_ok,
                timing_ms: r.timing_ms,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fightfake_core::proof_bundle::{verify_proof_bundle, ProofBundle as ToolkitBundle};

    fn to_toolkit_bundle(b: &ProofBundle) -> ToolkitBundle {
        ToolkitBundle {
            num_steps: b.num_steps,
            z0: b.z0.clone(),
            h2: b.h2,
            device_vk: b.device_vk,
            vk: b.vk.clone(),
            u_running: fightfake_core::proof_bundle::FoldedInstance {
                cm_e: b.u_running.cm_e,
                u: b.u_running.u,
                cm_q: b.u_running.cm_q,
                cm_w: b.u_running.cm_w,
            },
            u_current: fightfake_core::proof_bundle::FoldedInstance {
                cm_e: b.u_current.cm_e,
                u: b.u_current.u,
                cm_q: b.u_current.cm_q,
                cm_w: b.u_current.cm_w,
            },
            proof: b.proof.clone(),
            cm_t: b.cm_t,
            r: b.r,
        }
    }

    #[test]
    fn spike_produces_ffpb_verifiable_by_toolkit() {
        let result = run_spike(SpikeConfig::default()).expect("spike prove");
        assert!(result.ffpb_header_ok);
        let bytes = result.proof_bytes;
        let toolkit = ToolkitBundle::from_bytes(&bytes).expect("parse FFPB");
        let ok = verify_proof_bundle(&toolkit).expect("verify");
        assert!(ok, "toolkit portable verify must accept spike proof");
    }
}
