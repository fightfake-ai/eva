//! Browser Nova IVC session: one step per JS call so the UI can paint.
//!
//! Wrap is **offline**, not [`video::decider::DeciderEthCircuit`]:
//! native `Nova::verify` (primary + CycleFold in their own fields) plus Spartan
//! on [`video::decider::NativePrimaryCircuit`] (~step-circuit size, `FpVar` only).
//! Camera σ is checked in ordinary code. See `docs/offline-decider-and-wasm-spartan.md`.
#![allow(dead_code)]

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::Arc;

use ark_bn254::{constraints::GVar, Fq, Fr, G1Projective as Projective};
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as GrumpkinProjective};
use ark_std::rand::SeedableRng;
use folding_schemes::{
    commitment::pedersen::Pedersen,
    folding::nova::Nova,
    frontend::FCircuit,
    transcript::poseidon::poseidon_test_config,
    FoldingScheme, MVM,
};
use rand::rngs::StdRng;
use serde::Serialize;
use video::decider::{
    decode_proof, decode_vk, dummy_native_primary, encode_proof, encode_vk,
    native_primary_circuit_id, native_primary_from_running, prove_native_primary,
    setup_native_primary, sha256_hex, verify_native_primary,
};

use crate::ivc_artifact::{decode_ivc, encode_ivc, peek_ivc};
use video::edit::constraints::{Brightness, BrightnessCfg, RedactRect, RedactRectCfg};
use video::encode::Matrix;
use video::griffin::params::GriffinParams;
use video::{
    rgb8_to_yuv420p, yuv420_to_macroblocks, EditOnlyCircuit, EditOnlyExternalInputs, MB_UV_BYTES,
    MB_Y_BYTES,
};

use super::{field_hex, spike_circuit as brightness_circuit};

const WASM_BPS_CAP: usize = 4;
const RNG_SEED: u64 = 1;

type BrightOp = Brightness;
type RedactOp = RedactRect;

type BrightNova = Nova<
    Projective,
    GVar,
    GrumpkinProjective,
    GVar2,
    EditOnlyCircuit<Fr, BrightOp>,
    Pedersen<Projective>,
    Pedersen<GrumpkinProjective>,
>;

type RedactNova = Nova<
    Projective,
    GVar,
    GrumpkinProjective,
    GVar2,
    EditOnlyCircuit<Fr, RedactOp>,
    Pedersen<Projective>,
    Pedersen<GrumpkinProjective>,
>;

type BrightParams = (
    <BrightNova as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, BrightOp>>>::ProverParam,
    <BrightNova as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, BrightOp>>>::VerifierParam,
);

type RedactParams = (
    <RedactNova as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, RedactOp>>>::ProverParam,
    <RedactNova as FoldingScheme<Projective, GrumpkinProjective, EditOnlyCircuit<Fr, RedactOp>>>::VerifierParam,
);

type Block = (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>);

#[derive(Serialize)]
pub struct SessionInfo {
    pub phase: &'static str,
    pub width: usize,
    pub height: usize,
    pub mbs: usize,
    pub bps: usize,
    pub steps: usize,
    pub step: usize,
}

#[derive(Serialize)]
pub struct SessionDone {
    pub h1: String,
    pub h2: String,
    pub steps: usize,
    pub gadget: &'static str,
    pub bps: usize,
    pub circuit_id: String,
    pub proof_system: &'static str,
    pub verified: bool,
    pub ivc_verified: bool,
    pub proof_bytes_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ivc_bytes_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vk_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_constraints: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_error: Option<String>,
}

enum Kind {
    Bright {
        params: BrightParams,
        fs: BrightNova,
        blocks: Vec<Block>,
        scale: u16,
        bps: usize,
        steps: usize,
        step: usize,
        sk: Fq,
    },
    Redact {
        params: RedactParams,
        fs: RedactNova,
        blocks: Vec<Block>,
        cfgs: Vec<RedactRectCfg>,
        bps: usize,
        steps: usize,
        step: usize,
        sk: Fq,
    },
}

thread_local! {
    static SESSION: RefCell<Option<Kind>> = RefCell::new(None);
    static LAST_PROOF: RefCell<Option<Vec<u8>>> = RefCell::new(None);
    static LAST_IVC: RefCell<Option<Vec<u8>>> = RefCell::new(None);
}

fn pick_bps(mbs: usize) -> Result<usize, String> {
    if mbs == 0 {
        return Err("image has no 16×16 macroblocks".into());
    }
    for b in [WASM_BPS_CAP, 2, 1] {
        if mbs % b == 0 {
            return Ok(b);
        }
    }
    Err(format!("{mbs} macroblocks not divisible by 1"))
}

fn rgba_to_rgb_cropped(rgba: &[u8], w: usize, h: usize) -> Result<(Vec<u8>, usize, usize), String> {
    if rgba.len() != w * h * 4 {
        return Err(format!(
            "expected {} RGBA bytes, got {}",
            w * h * 4,
            rgba.len()
        ));
    }
    if w < 16 || h < 16 {
        return Err("need at least 16×16".into());
    }
    let cw = (w / 16) * 16;
    let ch = (h / 16) * 16;
    let mut rgb = Vec::with_capacity(cw * ch * 3);
    for y in 0..ch {
        for x in 0..cw {
            let i = (y * w + x) * 4;
            rgb.extend_from_slice(&rgba[i..i + 3]);
        }
    }
    Ok((rgb, cw, ch))
}

fn pack_blocks(orig_y: &[u8], orig_u: &[u8], orig_v: &[u8]) -> Vec<Block> {
    let n = orig_y.len() / MB_Y_BYTES;
    (0..n)
        .map(|i| {
            (
                Matrix::from_vec(orig_y[i * MB_Y_BYTES..(i + 1) * MB_Y_BYTES].to_vec()),
                Matrix::from_vec(orig_u[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES].to_vec()),
                Matrix::from_vec(orig_v[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES].to_vec()),
            )
        })
        .collect()
}

fn redact_circuit() -> EditOnlyCircuit<Fr, RedactOp> {
    EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    }
}

fn build_redact_cfg(
    global_mb: usize,
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> RedactRectCfg {
    let cols = width / 16;
    let mb_x = global_mb % cols;
    let mb_y = global_mb / cols;
    let origin_x = mb_x * 16;
    let origin_y = mb_y * 16;
    let x1 = x.min(width);
    let y1 = y.min(height);
    let x2 = (x + w).min(width);
    let y2 = (y + h).min(height);
    RedactRectCfg::from_rectangle(origin_x, origin_y, true, x1, y1, x2, y2, 0)
}

fn tiles_from_rgba(rgba: &[u8], width: u32, height: u32) -> Result<(Vec<Block>, usize, usize), String> {
    let (rgb, cw, ch) = rgba_to_rgb_cropped(rgba, width as usize, height as usize)?;
    let yuv = rgb8_to_yuv420p(&rgb, cw, ch)?;
    let (oy, ou, ov) = yuv420_to_macroblocks(&yuv, cw, ch, 1)?;
    Ok((pack_blocks(&oy, &ou, &ov), cw, ch))
}

pub fn nova_start(
    rgba: &[u8],
    width: u32,
    height: u32,
    gadget: &str,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
) -> Result<String, String> {
    let (blocks, cw, ch) = tiles_from_rgba(rgba, width, height)?;
    let mbs = blocks.len();
    let bps = pick_bps(mbs)?;
    let steps = mbs / bps;
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    let poseidon_config = poseidon_test_config();
    let sk = Fq::rand(&mut rng);

    let kind = match gadget {
        "brightness" => {
            let scale = (a as u16).clamp(1, 2048);
            let f_circuit = brightness_circuit();
            let (pp, vp) = BrightNova::preprocess(
                &poseidon_config,
                &f_circuit,
                &mut rng,
                &EditOnlyExternalInputs {
                    blocks: blocks[0..bps].to_vec(),
                    edit_configs: (0..bps).map(|_| BrightnessCfg(scale)).collect(),
                },
            )
            .map_err(|e| format!("Nova preprocess: {e}"))?;
            let fs = BrightNova::init(&(pp.clone(), vp.clone()), f_circuit, vec![Fr::zero(), Fr::zero()])
                .map_err(|e| format!("Nova init: {e}"))?;
            Kind::Bright {
                params: (pp, vp),
                fs,
                blocks,
                scale,
                bps,
                steps,
                step: 0,
                sk,
            }
        }
        "redact" => {
            let cfgs: Vec<_> = (0..mbs)
                .map(|i| build_redact_cfg(i, cw, ch, a as usize, b as usize, c as usize, d as usize))
                .collect();
            let f_circuit = redact_circuit();
            let (pp, vp) = RedactNova::preprocess(
                &poseidon_config,
                &f_circuit,
                &mut rng,
                &EditOnlyExternalInputs {
                    blocks: blocks[0..bps].to_vec(),
                    edit_configs: cfgs[0..bps].to_vec(),
                },
            )
            .map_err(|e| format!("Nova preprocess: {e}"))?;
            let fs = RedactNova::init(&(pp.clone(), vp.clone()), f_circuit, vec![Fr::zero(), Fr::zero()])
                .map_err(|e| format!("Nova init: {e}"))?;
            Kind::Redact {
                params: (pp, vp),
                fs,
                blocks,
                cfgs,
                bps,
                steps,
                step: 0,
                sk,
            }
        }
        other => return Err(format!("unsupported gadget {other}")),
    };

    SESSION.with(|s| *s.borrow_mut() = Some(kind));
    serde_json::to_string(&SessionInfo {
        phase: "tiled",
        width: cw,
        height: ch,
        mbs,
        bps,
        steps,
        step: 0,
    })
    .map_err(|e| e.to_string())
}

pub fn nova_step() -> Result<String, String> {
    SESSION.with(|slot| {
        let mut guard = slot.borrow_mut();
        let sess = guard.as_mut().ok_or("no nova session")?;
        let (step, steps) = match sess {
            Kind::Bright {
                params,
                fs,
                blocks,
                scale,
                bps,
                steps,
                step,
                ..
            } => {
                if *step >= *steps {
                    return Err("nova already finished".into());
                }
                let start = *step * *bps;
                let end = start + *bps;
                fs.prove_step(
                    params,
                    &EditOnlyExternalInputs {
                        blocks: blocks[start..end].to_vec(),
                        edit_configs: (0..*bps).map(|_| BrightnessCfg(*scale)).collect(),
                    },
                )
                .map_err(|e| format!("Nova prove_step {step}: {e}"))?;
                *step += 1;
                (*step, *steps)
            }
            Kind::Redact {
                params,
                fs,
                blocks,
                cfgs,
                bps,
                steps,
                step,
                ..
            } => {
                if *step >= *steps {
                    return Err("nova already finished".into());
                }
                let start = *step * *bps;
                let end = start + *bps;
                fs.prove_step(
                    params,
                    &EditOnlyExternalInputs {
                        blocks: blocks[start..end].to_vec(),
                        edit_configs: cfgs[start..end].to_vec(),
                    },
                )
                .map_err(|e| format!("Nova prove_step {step}: {e}"))?;
                *step += 1;
                (*step, *steps)
            }
        };
        serde_json::to_string(&serde_json::json!({
            "phase": "nova_step",
            "step": step,
            "steps": steps,
        }))
        .map_err(|e| e.to_string())
    })
}

fn device_sigma(
    sk: Fq,
    z_i0: Fr,
    rng: &mut StdRng,
) -> Result<(GrumpkinProjective, (Fr, Fq)), String> {
    let poseidon_config = poseidon_test_config();
    let device_vk = GrumpkinProjective::generator() * sk;
    let (px, py) = {
        let p = device_vk.into_affine();
        p.xy().unwrap_or((Fr::zero(), Fr::zero()))
    };
    let r = Fq::rand(rng);
    let rx = (GrumpkinProjective::generator() * r)
        .into_affine()
        .x()
        .unwrap_or_default();
    let e = CRH::evaluate(&poseidon_config, [rx, px, py, z_i0]).map_err(|e| format!("Schnorr hash: {e}"))?;
    let sigma = (
        rx,
        r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()),
    );
    Ok((device_vk, sigma))
}

fn verify_device_sigma(vk: GrumpkinProjective, h1: Fr, sigma: (Fr, Fq)) -> Result<(), String> {
    let poseidon_config = poseidon_test_config();
    let (rx, s) = sigma;
    let (px, py) = {
        let p = vk.into_affine();
        p.xy().unwrap_or((Fr::zero(), Fr::zero()))
    };
    let e = CRH::evaluate(&poseidon_config, [rx, px, py, h1]).map_err(|e| format!("Schnorr hash: {e}"))?;
    let e_fq = Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le());
    let r = GrumpkinProjective::generator() * s - vk * e_fq;
    let rx_chk = r.into_affine().x().unwrap_or_default();
    if rx_chk != rx {
        return Err("camera σ did not verify".into());
    }
    Ok(())
}

type NovaFC<FC> = Nova<
    Projective,
    GVar,
    GrumpkinProjective,
    GVar2,
    FC,
    Pedersen<Projective>,
    Pedersen<GrumpkinProjective>,
>;

fn wrap_log(msg: &str) {
    #[cfg(all(feature = "wasm", target_arch = "wasm32"))]
    web_sys::console::log_1(&format!("[wrap] {msg}").into());
    let _ = msg;
}

struct WrapOutcome {
    ivc_verified: bool,
    spartan_verified: bool,
    proof_bytes: Vec<u8>,
    ivc_bytes: Vec<u8>,
    vk_sha256: Option<String>,
    num_constraints: Option<usize>,
    proof_system: &'static str,
    wrap_error: Option<String>,
}

/// Native IVC + native-field Spartan. Never builds `DeciderEthCircuit`.
fn wrap_offline<FC: FCircuit<Fr>>(
    fs: NovaFC<FC>,
    params: (
        <NovaFC<FC> as FoldingScheme<Projective, GrumpkinProjective, FC>>::ProverParam,
        <NovaFC<FC> as FoldingScheme<Projective, GrumpkinProjective, FC>>::VerifierParam,
    ),
    sk: Fq,
    rng: &mut StdRng,
    gadget: &'static str,
    bps: usize,
) -> WrapOutcome {
    let fail_ivc = |e: String| WrapOutcome {
        ivc_verified: false,
        spartan_verified: false,
        proof_bytes: Vec::new(),
        ivc_bytes: Vec::new(),
        vk_sha256: None,
        num_constraints: None,
        proof_system: "nova-ivc",
        wrap_error: Some(e),
    };
    let fail_wrap = |ivc_bytes: Vec<u8>, e: String| WrapOutcome {
        ivc_verified: true,
        spartan_verified: false,
        proof_bytes: Vec::new(),
        ivc_bytes,
        vk_sha256: None,
        num_constraints: None,
        proof_system: "nova-ivc",
        wrap_error: Some(e),
    };

    let z_i0 = fs.z_i[0];
    wrap_log("device σ");
    let (device_vk, sigma) = match device_sigma(sk, z_i0, rng) {
        Ok(v) => v,
        Err(e) => return fail_ivc(e),
    };
    if let Err(e) = verify_device_sigma(device_vk, z_i0, sigma) {
        return fail_ivc(e);
    }

    wrap_log("Nova::verify (primary + CycleFold)");
    let (run, cur, cf) = fs.instances();
    if let Err(e) = NovaFC::<FC>::verify(
        &params.1,
        fs.z_0.clone(),
        fs.z_i.clone(),
        fs.i,
        run.clone(),
        cur.clone(),
        cf.clone(),
    ) {
        return fail_ivc(format!("Nova::verify: {e}"));
    }

    let ivc_bytes = match encode_ivc(
        gadget,
        bps,
        &fs.z_0,
        &fs.z_i,
        fs.i,
        &run,
        &cur,
        &cf,
        &device_vk,
        &sigma,
    ) {
        Ok(b) => b,
        Err(e) => return fail_ivc(format!("encode IVC transcript: {e}")),
    };
    wrap_log(&format!("IVC transcript {} bytes", ivc_bytes.len()));

    wrap_log("Spartan prove NativePrimaryCircuit");
    let circuit = native_primary_from_running(
        params.1.r1cs.clone(),
        fs.U_i.u,
        fs.U_i.x.clone(),
        fs.W_i.QW.clone(),
        Fr::retrieve_e(&fs.E),
    );
    match prove_native_primary(circuit) {
        Ok((proof, vk)) => {
            wrap_log(&format!(
                "Spartan prove ok cons={} padded={} vars={}",
                vk.num_constraints, vk.num_constraints_padded, vk.num_vars
            ));
            match verify_native_primary(&vk, &proof) {
                Ok(true) => {
                    let proof_bytes = match encode_proof(&proof) {
                        Ok(b) => b,
                        Err(e) => return fail_wrap(ivc_bytes, format!("encode Spartan proof: {e}")),
                    };
                    let vk_sha256 = match encode_vk(&vk) {
                        Ok(b) => Some(sha256_hex(&b)),
                        Err(_) => None,
                    };
                    WrapOutcome {
                        ivc_verified: true,
                        spartan_verified: true,
                        proof_bytes,
                        ivc_bytes,
                        vk_sha256,
                        num_constraints: Some(vk.num_constraints),
                        proof_system: "nova-offline-spartan",
                        wrap_error: None,
                    }
                }
                Ok(false) => fail_wrap(ivc_bytes, "native-primary Spartan verify returned false".into()),
                Err(e) => fail_wrap(ivc_bytes, format!("native-primary Spartan verify: {e}")),
            }
        }
        Err(e) => fail_wrap(ivc_bytes, format!("native-primary Spartan prove: {e}")),
    }
}

/// Eva h1/h2 after Nova IVC, without the wrap.
pub fn nova_digest() -> Result<String, String> {
    SESSION.with(|slot| {
        let guard = slot.borrow();
        let sess = guard.as_ref().ok_or("no nova session")?;
        let (h1, h2, steps, step) = match sess {
            Kind::Bright {
                fs, steps, step, ..
            } => {
                let st = fs.state();
                (
                    field_hex(&st.first().copied().unwrap_or(Fr::zero())),
                    field_hex(&st.get(1).copied().unwrap_or(Fr::zero())),
                    *steps,
                    *step,
                )
            }
            Kind::Redact {
                fs, steps, step, ..
            } => {
                let st = fs.state();
                (
                    field_hex(&st.first().copied().unwrap_or(Fr::zero())),
                    field_hex(&st.get(1).copied().unwrap_or(Fr::zero())),
                    *steps,
                    *step,
                )
            }
        };
        serde_json::to_string(&serde_json::json!({
            "h1": h1,
            "h2": h2,
            "steps": steps,
            "step": step,
            "proof_system": "nova-wasm",
        }))
        .map_err(|e| e.to_string())
    })
}

pub fn nova_finish() -> Result<String, String> {
    LAST_PROOF.with(|slot| slot.borrow_mut().take());
    LAST_IVC.with(|slot| slot.borrow_mut().take());
    SESSION.with(|slot| {
        let sess = slot.borrow_mut().take().ok_or("no nova session")?;
        let mut rng = StdRng::seed_from_u64(RNG_SEED.wrapping_add(1));
        let (h1, h2, steps, gadget, bps, wrap) = match sess {
            Kind::Bright {
                fs,
                params,
                steps,
                step,
                sk,
                bps,
                ..
            } => {
                if step != steps {
                    return Err(format!("nova unfinished ({step}/{steps})"));
                }
                let st = fs.state();
                let h1 = field_hex(&st.first().copied().unwrap_or(Fr::zero()));
                let h2 = field_hex(&st.get(1).copied().unwrap_or(Fr::zero()));
                (
                    h1,
                    h2,
                    steps,
                    "brightness",
                    bps,
                    wrap_offline(fs, params, sk, &mut rng, "brightness", bps),
                )
            }
            Kind::Redact {
                fs,
                params,
                steps,
                step,
                sk,
                bps,
                ..
            } => {
                if step != steps {
                    return Err(format!("nova unfinished ({step}/{steps})"));
                }
                let st = fs.state();
                let h1 = field_hex(&st.first().copied().unwrap_or(Fr::zero()));
                let h2 = field_hex(&st.get(1).copied().unwrap_or(Fr::zero()));
                (
                    h1,
                    h2,
                    steps,
                    "redact",
                    bps,
                    wrap_offline(fs, params, sk, &mut rng, "redact", bps),
                )
            }
        };
        if !wrap.ivc_bytes.is_empty() {
            LAST_IVC.with(|slot| *slot.borrow_mut() = Some(wrap.ivc_bytes.clone()));
        }
        if !wrap.proof_bytes.is_empty() {
            LAST_PROOF.with(|slot| *slot.borrow_mut() = Some(wrap.proof_bytes.clone()));
        }
        serde_json::to_string(&SessionDone {
            h1,
            h2,
            steps,
            gadget,
            bps,
            circuit_id: native_primary_circuit_id(gadget, bps),
            proof_system: wrap.proof_system,
            verified: wrap.ivc_verified && wrap.spartan_verified,
            ivc_verified: wrap.ivc_verified,
            proof_bytes_len: wrap.proof_bytes.len(),
            ivc_bytes_len: (!wrap.ivc_bytes.is_empty()).then_some(wrap.ivc_bytes.len()),
            vk_sha256: wrap.vk_sha256,
            num_constraints: wrap.num_constraints,
            wrap_error: wrap.wrap_error,
        })
        .map_err(|e| e.to_string())
    })
}

/// Binary Spartan proof left by the last successful [`nova_finish`]. Empty if none.
pub fn nova_take_proof() -> Vec<u8> {
    LAST_PROOF.with(|slot| slot.borrow_mut().take().unwrap_or_default())
}

/// `FFIV1` Nova transcript (instances + witnesses + σ) from the last successful IVC.
pub fn nova_take_ivc() -> Vec<u8> {
    LAST_IVC.with(|slot| slot.borrow_mut().take().unwrap_or_default())
}

fn dummy_block() -> Block {
    (
        Matrix::from_vec(vec![80u8; MB_Y_BYTES]),
        Matrix::from_vec(vec![128u8; MB_UV_BYTES]),
        Matrix::from_vec(vec![128u8; MB_UV_BYTES]),
    )
}

fn spartan_binds_running(proof_bytes: &[u8], u: Fr, x: &[Fr]) -> Result<bool, String> {
    let proof = decode_proof(proof_bytes).map_err(|e| format!("decode proof: {e}"))?;
    let io = proof.public_io_le_bytes();
    let mut expect = vec![u];
    expect.extend_from_slice(x);
    if io.len() != expect.len() {
        return Err(format!(
            "Spartan public IO len {} != running instance IO {}",
            io.len(),
            expect.len()
        ));
    }
    Ok(io.iter().zip(expect.iter()).all(|(got, f)| {
        let want = f.into_bigint().to_bytes_le();
        let n = got.len().min(want.len());
        got[..n] == want[..n]
            && got[n..].iter().all(|b| *b == 0)
            && want[n..].iter().all(|b| *b == 0)
    }))
}

/// Reconstruct Nova verifier params for `gadget` at `bps` (shape depends on those, not the photo).
fn preprocess_bright(bps: usize) -> Result<BrightParams, String> {
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    let poseidon_config = poseidon_test_config();
    let _sk = Fq::rand(&mut rng);
    let f_circuit = brightness_circuit();
    BrightNova::preprocess(
        &poseidon_config,
        &f_circuit,
        &mut rng,
        &EditOnlyExternalInputs {
            blocks: (0..bps).map(|_| dummy_block()).collect(),
            edit_configs: (0..bps).map(|_| BrightnessCfg(1024)).collect(),
        },
    )
    .map_err(|e| format!("Nova preprocess: {e}"))
}

fn preprocess_redact(bps: usize) -> Result<RedactParams, String> {
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    let poseidon_config = poseidon_test_config();
    let _sk = Fq::rand(&mut rng);
    let f_circuit = redact_circuit();
    let width = 16 * bps;
    let height = 16;
    let cfgs: Vec<_> = (0..bps)
        .map(|i| build_redact_cfg(i, width, height, 0, 0, 16, 16))
        .collect();
    RedactNova::preprocess(
        &poseidon_config,
        &f_circuit,
        &mut rng,
        &EditOnlyExternalInputs {
            blocks: (0..bps).map(|_| dummy_block()).collect(),
            edit_configs: cfgs,
        },
    )
    .map_err(|e| format!("Nova preprocess: {e}"))
}

/// Full third-party check: `Nova::verify` (CycleFold included), camera σ, Spartan, IO bind.
pub fn nova_verify_full(ivc_bytes: &[u8], vk_bytes: &[u8], proof_bytes: &[u8]) -> Result<String, String> {
    let (gadget, bps) = peek_ivc(ivc_bytes)?;
    wrap_log(&format!("full verify gadget={gadget} bps={bps}"));
    wrap_log("Nova preprocess (circuit shape)");

    let (nova_ok, sigma_ok, z_0, z_i, run_u, run_x) = match gadget.as_str() {
        "brightness" => {
            let (_pp, vp) = preprocess_bright(bps)?;
            let art = decode_ivc(ivc_bytes, &vp.r1cs, &vp.cf_r1cs)?;
            if art.gadget != gadget || art.bps != bps {
                return Err("IVC header mismatch".into());
            }
            let run_u = art.running.0.u;
            let run_x = art.running.0.x.clone();
            wrap_log("Nova::verify");
            let nova_ok = BrightNova::verify(
                &vp,
                art.z_0.clone(),
                art.z_i.clone(),
                art.i,
                art.running,
                art.incoming,
                art.cyclefold,
            )
            .map_err(|e| format!("Nova::verify: {e}"))
            .map(|()| true)?;
            let h1 = art.z_i.first().copied().unwrap_or(Fr::zero());
            let sigma_ok = verify_device_sigma(art.device_vk, h1, art.sigma).is_ok();
            (nova_ok, sigma_ok, art.z_0, art.z_i, run_u, run_x)
        }
        "redact" => {
            let (_pp, vp) = preprocess_redact(bps)?;
            let art = decode_ivc(ivc_bytes, &vp.r1cs, &vp.cf_r1cs)?;
            if art.gadget != gadget || art.bps != bps {
                return Err("IVC header mismatch".into());
            }
            let run_u = art.running.0.u;
            let run_x = art.running.0.x.clone();
            wrap_log("Nova::verify");
            let nova_ok = RedactNova::verify(
                &vp,
                art.z_0.clone(),
                art.z_i.clone(),
                art.i,
                art.running,
                art.incoming,
                art.cyclefold,
            )
            .map_err(|e| format!("Nova::verify: {e}"))
            .map(|()| true)?;
            let h1 = art.z_i.first().copied().unwrap_or(Fr::zero());
            let sigma_ok = verify_device_sigma(art.device_vk, h1, art.sigma).is_ok();
            (nova_ok, sigma_ok, art.z_0, art.z_i, run_u, run_x)
        }
        other => return Err(format!("unsupported gadget {other}")),
    };

    wrap_log("Spartan verify");
    let vk = decode_vk(vk_bytes).map_err(|e| format!("decode VK: {e}"))?;
    let proof = decode_proof(proof_bytes).map_err(|e| format!("decode proof: {e}"))?;
    let spartan_ok = verify_native_primary(&vk, &proof).map_err(|e| format!("Spartan: {e}"))?;
    let bind_ok = spartan_binds_running(proof_bytes, run_u, &run_x)?;

    serde_json::to_string(&serde_json::json!({
        "ok": nova_ok && sigma_ok && spartan_ok && bind_ok,
        "nova_ivc": nova_ok,
        "cyclefold": nova_ok,
        "camera_sigma": sigma_ok,
        "spartan": spartan_ok,
        "spartan_binds_running": bind_ok,
        "h1": field_hex(&z_i.first().copied().unwrap_or(Fr::zero())),
        "h2": field_hex(&z_i.get(1).copied().unwrap_or(Fr::zero())),
        "z0_h1": field_hex(&z_0.first().copied().unwrap_or(Fr::zero())),
        "gadget": gadget,
        "bps": bps,
    }))
    .map_err(|e| e.to_string())
}

/// Verify a canonical `FFSP1` proof against a published `FFSV1` VK.
pub fn spartan_verify(vk_bytes: &[u8], proof_bytes: &[u8]) -> Result<bool, String> {
    let vk = decode_vk(vk_bytes).map_err(|e| format!("decode VK: {e}"))?;
    let proof = decode_proof(proof_bytes).map_err(|e| format!("decode proof: {e}"))?;
    verify_native_primary(&vk, &proof).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct VkExportMeta {
    pub circuit_id: String,
    pub gadget: String,
    pub bps: usize,
    pub vk_sha256: String,
    pub vk_bytes_len: usize,
    pub num_constraints: usize,
    pub num_constraints_padded: usize,
    pub num_vars: usize,
    pub num_io: usize,
}

/// After [`nova_start`], synthesize the wrap VK (no Nova steps required).
pub fn nova_export_vk() -> Result<(Vec<u8>, VkExportMeta), String> {
    SESSION.with(|slot| {
        let guard = slot.borrow();
        let sess = guard.as_ref().ok_or("no nova session")?;
        let (gadget, bps, r1cs) = match sess {
            Kind::Bright { params, bps, .. } => ("brightness", *bps, params.1.r1cs.clone()),
            Kind::Redact { params, bps, .. } => ("redact", *bps, params.1.r1cs.clone()),
        };
        let vk = setup_native_primary(dummy_native_primary(r1cs))
            .map_err(|e| format!("Spartan setup: {e}"))?;
        let vk_bytes = encode_vk(&vk).map_err(|e| format!("encode VK: {e}"))?;
        let meta = VkExportMeta {
            circuit_id: native_primary_circuit_id(gadget, bps),
            gadget: gadget.to_string(),
            bps,
            vk_sha256: sha256_hex(&vk_bytes),
            vk_bytes_len: vk_bytes.len(),
            num_constraints: vk.num_constraints,
            num_constraints_padded: vk.num_constraints_padded,
            num_vars: vk.num_vars,
            num_io: vk.num_io,
        };
        Ok((vk_bytes, meta))
    })
}

pub fn spartan_tiny_smoke() -> Result<String, String> {
    let smoke = video::decider::spartan_tiny_smoke().map_err(|e| e.to_string())?;
    serde_json::to_string(&smoke).map_err(|e| e.to_string())
}
