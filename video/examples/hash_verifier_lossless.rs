//! Native h2 for the **lossless** Eva path: hash edited macroblock YUV (no preds/coeffs).
//!
//! Pair with `hash_recorder` (h1 over original pixels) and an `EditOnlyCircuit` Nova proof.
//! The printed value should match the IVC final state's `z[1]` when using the same edit config.
//!
//! ```bash
//! export DATA_PATH=/path/to/data_parsed
//! cargo run --release -p video --example hash_verifier_lossless
//! ```

use std::{path::Path, sync::Arc};

use ark_bn254::Fr;
use ark_std::{end_timer, start_timer, Zero};
use rayon::prelude::*;
use video::{
    edit::constraints::{Brightness, BrightnessCfg, EditConfig},
    griffin::{
        griffin::{Griffin, Permutation},
        params::GriffinParams,
    },
    hash_edited_macroblock, parse_prover_data,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let blocks_per_step = 256;
    let data = Path::new(env!("DATA_PATH")).join("foreman");
    let (blocks, _, _, _) = parse_prover_data(data.clone(), data, None)?;
    let num_steps = blocks.len() / blocks_per_step;

    let brightness = BrightnessCfg(416);
    let edit_configs = vec![brightness; blocks.len()];

    let griffin = Griffin::new(&Arc::new(GriffinParams::new(16, 5, 9)));
    let mut h = Fr::zero();

    let timer = start_timer!(|| "Hash edited macroblocks (lossless h2)");
    for i in 0..num_steps {
        if edit_configs[i * blocks_per_step..(i + 1) * blocks_per_step]
            .iter()
            .any(|cfg| cfg.should_keep_native())
        {
            h = griffin.hash(
                &(i * blocks_per_step..(i + 1) * blocks_per_step)
                    .into_par_iter()
                    .map(|j| {
                        hash_edited_macroblock::<Fr, Brightness>(
                            &griffin,
                            &blocks[j],
                            &edit_configs[j],
                        )
                    })
                    .chain(vec![h])
                    .collect::<Vec<_>>(),
            );
        }
    }
    end_timer!(timer);
    println!("{h}");

    Ok(())
}
