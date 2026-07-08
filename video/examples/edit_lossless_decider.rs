//! Full Eva **lossless** pipeline: Nova IVC over `EditOnlyCircuit` + Groth16 decider.
//!
//! Proves edit on macroblock YUV without H.264 encode constraints. Only needs
//! original pixels from `DATA_PATH/foreman` (no preds/coeffs from a re-encoded folder).
//!
//! ```bash
//! export DATA_PATH=/path/to/data_parsed
//! QUICK=1 cargo run --release -p video --example edit_lossless_decider
//! ```

#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]

use std::marker::PhantomData;
use std::{path::Path, sync::Arc};

use ark_bn254::Bn254;
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_groth16::Groth16;
use ark_snark::SNARK;
use ark_std::{add_to_trace, end_timer, start_timer};
use rand::thread_rng;
use video::decider::{Decider, DeciderEthCircuit};
use video::edit::constraints::{Brightness, BrightnessCfg};
use video::griffin::params::GriffinParams;
use video::utils::srs_size;
use video::{parse_orig_blocks, EditOnlyCircuit, EditOnlyExternalInputs};

use ark_bn254::{constraints::GVar, Fq, Fr, G1Projective as Projective};
use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use folding_schemes::{
    commitment::pedersen::Pedersen, folding::nova::Nova,
    transcript::poseidon::poseidon_test_config, FoldingScheme,
};

type Op = Brightness;

const BLOCKS_PER_STEP: usize = 256;

type NOVA = Nova<
    Projective,
    GVar,
    Projective2,
    GVar2,
    EditOnlyCircuit<Fr, Op>,
    Pedersen<Projective>,
    Pedersen<Projective2>,
>;

fn blocks_per_step() -> usize {
    if std::env::var("QUICK").is_ok() {
        4
    } else {
        BLOCKS_PER_STEP
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rng = &mut thread_rng();
    let sk = Fq::rand(rng);
    let blocks_per_step = blocks_per_step();
    let data = Path::new(env!("DATA_PATH")).join("foreman");

    println!("=== Lossless encoding proof (brightness edit, no H.264 encode) ===");

    let blocks = parse_orig_blocks(&data)?;
    let available_steps = blocks.len() / blocks_per_step;
    let num_steps = if std::env::var("QUICK").is_ok() {
        2.min(available_steps)
    } else {
        std::env::var("NUM_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(available_steps)
    };
    println!("blocks_per_step={blocks_per_step}, num_steps={num_steps}");

    let f_circuit = EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };

    let poseidon_config = poseidon_test_config();
    let brightness = BrightnessCfg(416);

    println!("Prepare Nova ProverParams & VerifierParams");
    let (pp, vp) = NOVA::preprocess(
        &poseidon_config,
        &f_circuit,
        rng,
        &EditOnlyExternalInputs {
            blocks: blocks[0..blocks_per_step].to_vec(),
            edit_configs: vec![brightness.clone(); blocks_per_step],
        },
    )?;

    let pk = Groth16::<Bn254>::generate_random_parameters_with_reduction(
        DeciderEthCircuit::<Projective, GVar, Projective2, GVar2> {
            _gc1: std::marker::PhantomData,
            _gc2: std::marker::PhantomData,
            r1cs: vp.r1cs.clone(),
            cf_r1cs: vp.cf_r1cs.clone(),
            cf_pedersen_params: pp.cf_cs_params.clone(),
            poseidon_config: poseidon_config.clone(),
            i: None,
            z_0: Some(vec![Fr::rand(rng), Fr::rand(rng)]),
            u_i: None,
            U_i: None,
            W_i1: None,
            cmT: None,
            r: None,
            cf_U_i: None,
            cf_W_i: None,
            E: None,
            cf_E: None,
            sigma: (Fr::rand(rng), Fq::rand(rng)),
            vk: Projective2::rand(rng),
            h1: Fr::rand(rng),
            h2: Fr::rand(rng),
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
        rng,
    )?;
    add_to_trace!(|| "SRS size", || format!("{}", srs_size(&pk, &pp.cs_params)));
    if std::env::var("SETUP_ONLY").is_ok() {
        return Ok(());
    }

    let decider_vp = Groth16::<Bn254>::process_vk(&pk.vk)?;
    let decider_pp = pk;

    let (circuit, initial_state, last_state, running_instance, incoming_instance) = {
        let params = (pp, vp);
        let initial_state = vec![Fr::zero(), Fr::zero()];

        println!("Initialize FoldingScheme");
        let mut folding_scheme = NOVA::init(&params, f_circuit, initial_state.clone())?;

        let timer = start_timer!(|| "IVC::prove");
        for i in 0..num_steps {
            let timer = start_timer!(|| format!("Nova::prove_step {i}"));
            folding_scheme.prove_step(
                &params,
                &EditOnlyExternalInputs {
                    blocks: blocks[i * blocks_per_step..(i + 1) * blocks_per_step].to_vec(),
                    edit_configs: vec![brightness.clone(); blocks_per_step],
                },
            )?;
            end_timer!(timer);
        }
        end_timer!(timer);

        let last_state = folding_scheme.state();
        add_to_trace!(|| format!("state at last ({num_steps}-th) step"), || format!("{last_state:?}"));

        let (running_instance, incoming_instance, cyclefold_instance) = folding_scheme.instances();
        println!("Run Nova IVC verifier");
        NOVA::verify(
            &params.1,
            initial_state.clone(),
            last_state.clone(),
            Fr::from(num_steps as u32),
            running_instance.clone(),
            incoming_instance.clone(),
            cyclefold_instance,
        )?;

        let vk = Projective2::generator() * sk;
        let (px, py) = {
            let p = vk.into_affine();
            p.xy().unwrap_or((Fr::zero(), Fr::zero()))
        };
        let sigma = {
            let r = Fq::rand(rng);
            let rx = (Projective2::generator() * r)
                .into_affine()
                .x()
                .unwrap_or_default();
            let e = CRH::evaluate(&poseidon_config, [rx, px, py, folding_scheme.z_i[0]])?;
            (
                rx,
                r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()),
            )
        };

        let U_i = folding_scheme.U_i.clone();

        let circuit = DeciderEthCircuit::<Projective, GVar, Projective2, GVar2>::from_nova(
            folding_scheme,
            params,
            vk,
            sigma,
        )?;

        let u_i = circuit.u_i.clone().unwrap();

        (
            circuit,
            initial_state,
            last_state,
            U_i,
            u_i,
        )
    };

    let vk = Projective2::generator() * sk;
    let proof = Decider::prove(decider_pp, rng, circuit)?;

    let start = start_timer!(|| "Decider verify");
    let verified = Decider::verify(
        decider_vp,
        vk,
        Fr::from(num_steps as u32),
        initial_state,
        last_state[1],
        &running_instance,
        &incoming_instance,
        proof,
    )?;
    assert!(verified);
    end_timer!(start);

    println!("Lossless encoding proof verified (Nova + Groth16 decider).");
    println!("h1 (recorder binding) is signed via decider; h2 matches hash_verifier_lossless.");

    Ok(())
}
