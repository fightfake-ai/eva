//! Lossless encoding demo: brightness edit on macroblock YUV without H.264 encode proof.
//!
//! This is the Nova-only smoke test; see `edit_lossless_decider` for the full decider pipeline
//! and `hash_verifier_lossless` for the native h2 check over edited pixels.
//!
//! # Usage
//!
//! ```bash
//! export DATA_PATH=/path/to/data_parsed
//!
//! # Quick smoke (4 blocks/step, 2 IVC steps) — seconds
//! QUICK=1 cargo run --release -p video --example edit_bright_only
//!
//! # Full blocks/step (256) — same scale as production examples
//! cargo run --release -p video --example edit_bright_only
//! ```
//!
//! Environment:
//! - `DATA_PATH` — compile-time path to `data_parsed/` (see repo README)
//! - `QUICK=1` — use 4 blocks per step and 2 steps
//! - `NUM_STEPS` — IVC steps when not `QUICK` (default: min available, capped at 4 for demo)

#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use ark_bn254::{constraints::GVar, Fr, G1Projective as Projective};
use ark_ff::Zero;
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use ark_std::{end_timer, start_timer};
use folding_schemes::{
    commitment::pedersen::Pedersen,
    folding::nova::Nova,
    transcript::poseidon::poseidon_test_config,
    FoldingScheme,
};
use rand::thread_rng;
use video::edit::constraints::{Brightness, BrightnessCfg};
use video::griffin::params::GriffinParams;
use video::{parse_orig_blocks, EditOnlyCircuit, EditOnlyExternalInputs};

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
    let blocks_per_step = blocks_per_step();
    let video_name = std::env::var("VIDEO").unwrap_or_else(|_| "foreman".into());
    let data = Path::new(env!("DATA_PATH")).join(&video_name);

    println!("=== Lossless encoding proof (brightness, Nova only) ===");
    println!("blocks_per_step={blocks_per_step}");
    println!("data={}", data.display());

    let blocks = parse_orig_blocks(&data)?;
    let available_steps = blocks.len() / blocks_per_step;
    let num_steps = if std::env::var("QUICK").is_ok() {
        2.min(available_steps)
    } else {
        std::env::var("NUM_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4.min(available_steps))
    };
    println!("num_steps={num_steps} (available={available_steps})");

    let f_circuit = EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };

    let poseidon_config = poseidon_test_config();
    let brightness = BrightnessCfg(416);

    println!("Nova preprocess (edit-only circuit)...");
    let timer = start_timer!(|| "preprocess");
    let (pp, vp) = NOVA::preprocess(
        &poseidon_config,
        &f_circuit,
        rng,
        &EditOnlyExternalInputs {
            blocks: blocks[0..blocks_per_step].to_vec(),
            edit_configs: vec![brightness.clone(); blocks_per_step],
        },
    )?;
    end_timer!(timer);

    let params = (pp, vp);
    let initial_state = vec![Fr::zero(), Fr::zero()];
    let mut folding_scheme = NOVA::init(&params, f_circuit, initial_state.clone())?;

    let prove_timer = start_timer!(|| "IVC prove (edit-only)");
    for i in 0..num_steps {
        let step_timer = start_timer!(|| format!("prove_step {i}"));
        folding_scheme.prove_step(
            &params,
            &EditOnlyExternalInputs {
                blocks: blocks[i * blocks_per_step..(i + 1) * blocks_per_step].to_vec(),
                edit_configs: vec![brightness.clone(); blocks_per_step],
            },
        )?;
        end_timer!(step_timer);
    }
    end_timer!(prove_timer);

    let last_state = folding_scheme.state();
    println!("IVC final state: {last_state:?}");

    let verify_timer = start_timer!(|| "Nova verify");
    let (running_instance, incoming_instance, cyclefold_instance) = folding_scheme.instances();
    NOVA::verify(
        &params.1,
        initial_state,
        last_state,
        Fr::from(num_steps as u32),
        running_instance,
        incoming_instance,
        cyclefold_instance,
    )?;
    end_timer!(verify_timer);

    println!("Lossless encoding Nova proof verified successfully.");
    println!();
    println!("This prover does NOT write an edited video file.");
    println!("To export a playable edited video (same brightness as this proof):");
    println!("  BRIGHTNESS=416 cargo run --release -p video --example macroblocks_to_yuv -- \\");
    println!("    data_parsed/{video_name} edited.yuv <width> <height> <frames>");
    println!("  ffplay -f rawvideo -pix_fmt yuv420p -s <width>x<height> edited.yuv");
    println!();
    println!("Other next steps:");
    println!("  hash_verifier_lossless  — native h2 over edited pixels (compare to state[1])");
    println!("  edit_lossless_decider   — full Nova + Groth16 decider");

    Ok(())
}
