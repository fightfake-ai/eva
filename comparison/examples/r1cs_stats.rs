//! Print R1CS dimension statistics for Eva's step and augmented circuits.
//!
//! # Usage
//!
//! ```bash
//! BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats
//! ```
//!
//! See `docs/spartan2-comparison/METRICS.md` for how to record results.

use comparison::{
    eva_step::{blocks_per_step_from_env, synthesize_both_r1cs},
    print_r1cs_stats, stats_from_r1cs,
};

fn main() {
    let blocks_per_step = blocks_per_step_from_env();
    println!("Synthesizing Eva circuits with BLOCKS_PER_STEP={blocks_per_step}...");
    println!("(This may take a minute on first run — constraint synthesis is expensive.)\n");

    let (step_r1cs, augmented_r1cs) = synthesize_both_r1cs(blocks_per_step)
        .unwrap_or_else(|e| panic!("circuit synthesis failed: {e}"));

    let stats = [
        stats_from_r1cs("eva-step-only", &step_r1cs),
        stats_from_r1cs("eva-augmented", &augmented_r1cs),
    ];

    print_r1cs_stats(&stats);

    println!();
    println!("Notes:");
    println!("  - 'eva-step-only' = EditEncodeCircuit + lookup arguments (the F circuit).");
    println!("  - 'eva-augmented' = full Nova AugmentedFCircuit (what prove_step actually folds).");
    println!("  - Use these constraint counts to size Phase 1 bellpepper placeholders.");
    println!("  - Record output in docs/spartan2-comparison/results/phase0.md");
}
