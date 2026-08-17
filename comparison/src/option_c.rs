//! Option C: keep Nova IVC + lookups; compare Groth16 decider vs transparent Spartan.
//!
//! | Path | Final proof | Covers | Trusted setup? |
//! |------|-------------|--------|----------------|
//! | **Groth16** | `video::decider` | Primary + CycleFold + device sigma | Yes |
//! | **Spartan full** | matrix-native Spartan on `DeciderEthCircuit` R1CS | **Same statement as Groth16** | No |
//! | **Spartan primary** | bellpepper relaxed check | Primary R1CS only | No |
//!
//! Full transparent path (default Spartan mode): synthesize the same
//! `DeciderEthCircuit` Groth16 uses, remap ark columns → Spartan layout, prove with
//! vendored `RelaxedR1CSSpartanProof` (`third_party/spartan2`) plus commitment
//! binding in the Fiat–Shamir transcript.
//!
//! ```bash
//! cargo run --release -p comparison --example phase3_option_c
//! SKIP_G16=1 SPARTAN_MODE=full cargo run --release -p comparison --example phase3_option_c
//! SKIP_G16=1 SPARTAN_MODE=primary cargo run --release -p comparison --example phase3_option_c
//! SKIP_SPARTAN=1 cargo run --release -p comparison --example phase3_option_c
//! ```

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use ark_bn254::{constraints::GVar, Bn254, Fq, Fr, G1Projective as Projective};
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::{BigInteger, PrimeField as ArkPrimeField, UniformRand, Zero};
use ark_groth16::Groth16;
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use ark_serialize::{CanonicalSerialize, Compress};
use ark_snark::SNARK;
use ff::PrimeField as FfPrimeField;
use folding_schemes::ccs::r1cs::R1CS;
use folding_schemes::commitment::pedersen::Pedersen;
use folding_schemes::folding::nova::circuits::ChallengeGadget;
use folding_schemes::folding::nova::nifs::NIFS;
use folding_schemes::folding::nova::traits::NovaR1CS;
use folding_schemes::folding::nova::{Nova, Witness};
use folding_schemes::transcript::poseidon::poseidon_test_config;
use folding_schemes::utils::vec::SparseMatrix;
use folding_schemes::{FoldingScheme, MVM};
use rand::thread_rng;
use serde::Serialize;
use spartan2::provider::Bn254Engine;
use spartan2::spartan_zk::SpartanZkSNARK;
use spartan2::traits::snark::R1CSSNARKTrait;
use spartan2::traits::Engine;
use video::decider::{Decider, DeciderEthCircuit};
use video::edit::constraints::{Brightness, BrightnessCfg};
use video::griffin::params::GriffinParams;
use video::{EditOnlyCircuit, EditOnlyExternalInputs};

use crate::ark_to_spartan::prove_ark_circuit;
use crate::bellpepper::relaxed_check::{
    RelaxedR1CSCheckCircuit, RelaxedR1CSWitness, SparseR1CS, SparseRow,
};

type E = Bn254Engine;
type NOVA = Nova<
    Projective,
    GVar,
    Projective2,
    GVar2,
    EditOnlyCircuit<Fr, Brightness>,
    Pedersen<Projective>,
    Pedersen<Projective2>,
>;
type DeciderCircuit = DeciderEthCircuit<Projective, GVar, Projective2, GVar2>;

/// Which Spartan final proof to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpartanMode {
    /// Full `DeciderEthCircuit` via ark→Spartan matrix path (transparent, complete statement).
    Full,
    /// Primary relaxed R1CS only (bellpepper re-encoding; incomplete vs Groth16).
    Primary,
}

impl SpartanMode {
    pub fn from_env() -> Self {
        match std::env::var("SPARTAN_MODE")
            .unwrap_or_else(|_| "full".into())
            .to_lowercase()
            .as_str()
        {
            "primary" | "prim" => Self::Primary,
            _ => Self::Full,
        }
    }
}

/// Config for Option C dual-path comparison.
#[derive(Clone, Debug)]
pub struct OptionCConfig {
    pub blocks_per_step: usize,
    pub num_steps: usize,
    pub run_groth16: bool,
    pub run_spartan: bool,
    pub spartan_mode: SpartanMode,
}

impl OptionCConfig {
    pub fn from_env() -> Self {
        Self {
            blocks_per_step: std::env::var("BLOCKS_PER_STEP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4),
            num_steps: std::env::var("NUM_STEPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4),
            run_groth16: std::env::var("SKIP_G16").is_err(),
            run_spartan: std::env::var("SKIP_SPARTAN").is_err(),
            spartan_mode: SpartanMode::from_env(),
        }
    }
}

/// Timings / sizes for one decider backend.
#[derive(Clone, Debug, Default)]
pub struct DeciderPathTimings {
    pub name: String,
    pub setup_ms: f64,
    pub prove_ms: f64,
    pub verify_ms: f64,
    pub proof_bytes: usize,
    pub notes: String,
}

/// Full Option C run result.
#[derive(Clone, Debug, Default)]
pub struct OptionCResult {
    pub blocks_per_step: usize,
    pub num_steps: usize,
    pub nova_preprocess_ms: f64,
    pub nova_ivc_ms: f64,
    pub primary_r1cs_constraints: usize,
    pub groth16: Option<DeciderPathTimings>,
    pub spartan: Option<DeciderPathTimings>,
}

fn synthetic_inputs(blocks: usize) -> EditOnlyExternalInputs<BrightnessCfg> {
    // Non-zero pixels so Pedersen commitments are not the identity (Decider verify
    // unwraps affine coords and panics on ∞).
    let block = (
        video::encode::Matrix::from_iter((0..256).map(|k| ((k * 17) % 256) as u8)),
        video::encode::Matrix::from_iter((0..64).map(|k| ((k * 5) % 256) as u8)),
        video::encode::Matrix::from_iter((0..64).map(|k| ((k * 9) % 256) as u8)),
    );
    EditOnlyExternalInputs {
        blocks: vec![block; blocks],
        edit_configs: vec![BrightnessCfg(416); blocks],
    }
}

fn ark_fr_to_spartan(x: Fr) -> <E as Engine>::Scalar {
    let bytes = x.into_bigint().to_bytes_le();
    let mut repr = <<E as Engine>::Scalar as FfPrimeField>::Repr::default();
    let dst: &mut [u8] = repr.as_mut();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
    <E as Engine>::Scalar::from_repr(repr).unwrap()
}

fn sparse_to_spartan(m: &SparseMatrix<Fr>) -> Vec<SparseRow<<E as Engine>::Scalar>> {
    m.coeffs
        .iter()
        .map(|row| {
            row.iter()
                .map(|(c, col)| (ark_fr_to_spartan(*c), *col))
                .collect()
        })
        .collect()
}

fn ark_r1cs_to_spartan(r1cs: &R1CS<Fr>) -> SparseR1CS<<E as Engine>::Scalar> {
    SparseR1CS {
        a: sparse_to_spartan(&r1cs.A),
        b: sparse_to_spartan(&r1cs.B),
        c: sparse_to_spartan(&r1cs.C),
        num_vars: r1cs.A.n_cols,
    }
}

fn bincode_size<T: Serialize>(v: &T) -> Result<usize, String> {
    bincode::serialize(v)
        .map(|b| b.len())
        .map_err(|e| format!("bincode: {e}"))
}

fn make_device_sigma(
    poseidon_config: &ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
    h1: Fr,
    rng: &mut impl rand::Rng,
) -> Result<(Projective2, (Fr, Fq)), String> {
    let sk = Fq::rand(rng);
    let vk = Projective2::generator() * sk;
    let (px, py) = {
        let p = vk.into_affine();
        p.xy().unwrap_or((Fr::zero(), Fr::zero()))
    };
    let r = Fq::rand(rng);
    let rx = (Projective2::generator() * r)
        .into_affine()
        .x()
        .unwrap_or_default();
    let e = CRH::evaluate(poseidon_config, [rx, px, py, h1])
        .map_err(|e| format!("sigma CRH: {e}"))?;
    let sigma = (
        rx,
        r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()),
    );
    Ok((vk, sigma))
}

/// Final fold (same as `DeciderEthCircuit::from_nova`) → primary `(U', W', E')`.
fn final_primary_fold(
    nova: &mut NOVA,
    pp: &folding_schemes::folding::nova::ProverParams<
        Projective,
        Projective2,
        Pedersen<Projective>,
        Pedersen<Projective2>,
    >,
    vp: &folding_schemes::folding::nova::VerifierParams<Projective, Projective2>,
) -> Result<(Witness<Projective>, Vec<Fr>, Fr, Vec<Fr>), String> {
    let cmT = NIFS::<Projective, Pedersen<Projective>>::compute_cmT(
        Some(&nova.stream),
        &pp.cs_params,
        &vp.r1cs,
        &nova.W_i,
        &nova.U_i,
        &nova.w_i,
        &nova.u_i,
        &nova.E,
        &mut nova.T,
    )
    .map_err(|e| format!("compute_cmT: {e}"))?;

    let (r_bits, cmT) = ChallengeGadget::<Projective>::get_challenge_device(
        Some(&nova.stream),
        Some(&nova.stream3),
        &nova.poseidon_config,
        &nova.U_i,
        &mut nova.u_i,
        &cmT,
        &nova.cmW,
    )
    .map_err(|e| format!("challenge: {e}"))?;
    let r = Fr::from_bigint(BigInteger::from_bits_le(&r_bits))
        .ok_or_else(|| "challenge out of bounds".to_string())?;

    let W_i1 = NIFS::<Projective, Pedersen<Projective>>::fold_witness(
        Some(&nova.stream),
        r,
        &nova.W_i,
        &nova.w_i,
        &mut nova.E,
        &nova.T,
    )
    .map_err(|e| format!("fold_witness: {e}"))?;
    nova.stream.synchronize().unwrap();

    let U_i1 = NIFS::<Projective, Pedersen<Projective>>::fold_committed_instance(
        r, &nova.U_i, &nova.u_i, &cmT,
    );
    let E = Fr::retrieve_e(&nova.E);
    let z = [vec![U_i1.u], U_i1.x.clone(), W_i1.QW.clone()].concat();

    vp.r1cs
        .check_relaxed_running_instance_relation(&W_i1, &U_i1, E.clone())
        .map_err(|e| format!("relaxed relation: {e}"))?;

    Ok((W_i1, E, U_i1.u, z))
}

fn run_nova_ivc(
    config: &OptionCConfig,
) -> Result<
    (
        NOVA,
        (
            folding_schemes::folding::nova::ProverParams<
                Projective,
                Projective2,
                Pedersen<Projective>,
                Pedersen<Projective2>,
            >,
            folding_schemes::folding::nova::VerifierParams<Projective, Projective2>,
        ),
        f64,
        f64,
        usize,
        Vec<Fr>,
        Vec<Fr>,
    ),
    String,
> {
    let mut rng = thread_rng();
    let f_circuit = EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };
    let poseidon_config = poseidon_test_config();
    let external = synthetic_inputs(config.blocks_per_step);

    let t0 = Instant::now();
    let (pp, vp) = NOVA::preprocess(&poseidon_config, &f_circuit, &mut rng, &external)
        .map_err(|e| format!("Nova preprocess: {e}"))?;
    let nova_preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let primary_r1cs_constraints = vp.r1cs.A.n_rows;

    let params = (pp, vp);
    let initial_state = vec![Fr::zero(), Fr::zero()];
    let mut nova = NOVA::init(&params, f_circuit, initial_state.clone())
        .map_err(|e| format!("Nova init: {e}"))?;

    let t_ivc = Instant::now();
    for _ in 0..config.num_steps {
        nova.prove_step(&params, &external)
            .map_err(|e| format!("prove_step: {e}"))?;
    }
    let nova_ivc_ms = t_ivc.elapsed().as_secs_f64() * 1000.0;

    let last_state = nova.state();
    let (running_instance, incoming_instance, cyclefold_instance) = nova.instances();
    NOVA::verify(
        &params.1,
        initial_state.clone(),
        last_state.clone(),
        Fr::from(config.num_steps as u32),
        running_instance,
        incoming_instance,
        cyclefold_instance,
    )
    .map_err(|e| format!("Nova verify: {e}"))?;

    Ok((
        nova,
        params,
        nova_preprocess_ms,
        nova_ivc_ms,
        primary_r1cs_constraints,
        initial_state,
        last_state,
    ))
}

/// Run Nova IVC then Groth16 and/or Spartan final proofs.
pub fn run_option_c(config: &OptionCConfig) -> Result<OptionCResult, String> {
    let mut rng = thread_rng();
    let poseidon_config = poseidon_test_config();

    let (
        mut nova,
        params,
        nova_preprocess_ms,
        nova_ivc_ms,
        primary_r1cs_constraints,
        initial_state,
        last_state,
    ) = run_nova_ivc(config)?;

    let mut result = OptionCResult {
        blocks_per_step: config.blocks_per_step,
        num_steps: config.num_steps,
        nova_preprocess_ms,
        nova_ivc_ms,
        primary_r1cs_constraints,
        ..Default::default()
    };

    // Device key + sigma only needed for the full DeciderEth statement.
    let need_full_circuit =
        config.run_groth16 || (config.run_spartan && config.spartan_mode == SpartanMode::Full);

    if need_full_circuit {
        let (vk, sigma) = make_device_sigma(&poseidon_config, nova.z_i[0], &mut rng)?;

        // Groth16 PK setup (dummy shape) — expensive; skip if SKIP_G16.
        let g16_pk = if config.run_groth16 {
            let t = Instant::now();
            let pk = Groth16::<Bn254>::generate_random_parameters_with_reduction(
                DeciderEthCircuit::<Projective, GVar, Projective2, GVar2> {
                    _gc1: PhantomData,
                    _gc2: PhantomData,
                    r1cs: params.1.r1cs.clone(),
                    cf_r1cs: params.1.cf_r1cs.clone(),
                    cf_pedersen_params: params.0.cf_cs_params.clone(),
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
                    vk: Projective2::rand(&mut rng),
                    h1: Fr::rand(&mut rng),
                    h2: Fr::rand(&mut rng),
                },
                vec![
                    (
                        &params.0.cs_params.generators[..params.1.r1cs.q],
                        params.0.cs_params.h.into_affine(),
                    ),
                    (
                        &params.0.cs_params.generators
                            [..params.1.r1cs.A.n_cols - 1 - params.1.r1cs.l - params.1.r1cs.q],
                        params.0.cs_params.h.into_affine(),
                    ),
                    (
                        &params.0.cs_params.generators[..params.1.r1cs.A.n_rows],
                        params.0.cs_params.h.into_affine(),
                    ),
                ],
                &mut rng,
            )
            .map_err(|e| format!("Groth16 setup: {e}"))?;
            Some((pk, t.elapsed().as_secs_f64() * 1000.0))
        } else {
            None
        };

        let U_i = nova.U_i.clone();
        let circuit = DeciderCircuit::from_nova(nova, params.clone(), vk, sigma)
            .map_err(|e| format!("from_nova: {e}"))?;
        let u_i = circuit.u_i.clone().unwrap();

        if let Some((pk, setup_ms)) = g16_pk {
            let decider_vp = Groth16::<Bn254>::process_vk(&pk.vk)
                .map_err(|e| format!("process_vk: {e}"))?;

            let t_prove = Instant::now();
            let proof = Decider::prove(pk, &mut rng, circuit.clone())
                .map_err(|e| format!("G16 prove: {e}"))?;
            let prove_ms = t_prove.elapsed().as_secs_f64() * 1000.0;

            let proof_bytes = {
                let (snark, cmT, r) = &proof;
                snark.serialized_size(Compress::Yes)
                    + cmT.serialized_size(Compress::Yes)
                    + r.serialized_size(Compress::Yes)
            };

            for (name, p) in [
                ("U.cmQ", U_i.cmQ),
                ("U.cmW", U_i.cmW),
                ("U.cmE", U_i.cmE),
                ("u.cmQ", u_i.cmQ),
                ("u.cmW", u_i.cmW),
                ("cmT", proof.1),
            ] {
                if p.is_zero() {
                    return Err(format!(
                        "Groth16 verify prep: {name} is identity (xy() would panic)"
                    ));
                }
            }

            let t_v = Instant::now();
            let ok = Decider::verify(
                decider_vp,
                vk,
                Fr::from(config.num_steps as u32),
                initial_state.clone(),
                last_state[1],
                &U_i,
                &u_i,
                proof,
            )
            .map_err(|e| format!("G16 verify: {e}"))?;
            if !ok {
                return Err("Groth16 decider verify returned false".into());
            }
            let verify_ms = t_v.elapsed().as_secs_f64() * 1000.0;

            result.groth16 = Some(DeciderPathTimings {
                name: "Groth16 DeciderEth".into(),
                setup_ms,
                prove_ms,
                verify_ms,
                proof_bytes,
                notes: "full: primary + CycleFold + sigma (trusted setup)".into(),
            });
        }

        if config.run_spartan && config.spartan_mode == SpartanMode::Full {
            result.spartan = Some(prove_spartan_full_decider(circuit)?);
        }
    } else if config.run_spartan && config.spartan_mode == SpartanMode::Primary {
        result.spartan = Some(prove_spartan_primary(&mut nova, &params.0, &params.1)?);
    }

    Ok(result)
}

fn prove_spartan_full_decider(circuit: DeciderCircuit) -> Result<DeciderPathTimings, String> {
    let (_proof, t) = prove_ark_circuit(circuit)?;
    Ok(DeciderPathTimings {
        name: "Spartan matrix (full DeciderEth)".into(),
        setup_ms: t.setup_ms,
        prove_ms: t.synthesize_ms + t.convert_ms + t.prove_ms,
        verify_ms: t.verify_ms,
        proof_bytes: t.proof_bytes,
        notes: format!(
            "transparent FULL statement (primary+CycleFold+sigma); \
             synthesize={:.1}ms convert={:.1}ms prove_core={:.1}ms; \
             cons={} (padded {}); vars={}; io={}",
            t.synthesize_ms,
            t.convert_ms,
            t.prove_ms,
            t.num_constraints,
            t.num_constraints_padded,
            t.num_vars,
            t.num_io
        ),
    })
}

fn prove_spartan_primary(
    nova: &mut NOVA,
    pp: &folding_schemes::folding::nova::ProverParams<
        Projective,
        Projective2,
        Pedersen<Projective>,
        Pedersen<Projective2>,
    >,
    vp: &folding_schemes::folding::nova::VerifierParams<Projective, Projective2>,
) -> Result<DeciderPathTimings, String> {
    let (_W, E, u, z) = final_primary_fold(nova, pp, vp)?;
    let sparse = ark_r1cs_to_spartan(&vp.r1cs);
    let witness = RelaxedR1CSWitness {
        z: z.iter().copied().map(ark_fr_to_spartan).collect(),
        e: E.iter().copied().map(ark_fr_to_spartan).collect(),
        u: ark_fr_to_spartan(u),
    };
    let circuit = RelaxedR1CSCheckCircuit::<E>::new(sparse, Some(witness));
    let proto = RelaxedR1CSCheckCircuit::<E>::new(ark_r1cs_to_spartan(&vp.r1cs), None);

    let t_setup = Instant::now();
    let (pk, vk) = SpartanZkSNARK::<E>::setup(proto).map_err(|e| format!("Spartan setup: {e}"))?;
    let setup_ms = t_setup.elapsed().as_secs_f64() * 1000.0;

    let t_prove = Instant::now();
    let prep = SpartanZkSNARK::<E>::prep_prove(&pk, circuit.clone(), false)
        .map_err(|e| format!("prep: {e}"))?;
    let (proof, _) = SpartanZkSNARK::<E>::prove(&pk, circuit, prep, false)
        .map_err(|e| format!("Spartan prove: {e}"))?;
    let prove_ms = t_prove.elapsed().as_secs_f64() * 1000.0;
    let proof_bytes = bincode_size(&proof)?;

    let t_v = Instant::now();
    let _io = proof
        .verify(&vk)
        .map_err(|e| format!("Spartan verify: {e}"))?;
    let verify_ms = t_v.elapsed().as_secs_f64() * 1000.0;

    Ok(DeciderPathTimings {
        name: "SpartanZk (primary relaxed R1CS)".into(),
        setup_ms,
        prove_ms,
        verify_ms,
        proof_bytes,
        notes: format!(
            "PRIMARY ONLY (no CycleFold/sigma); bellpepper re-encoding of {} rows",
            vp.r1cs.A.n_rows
        ),
    })
}
