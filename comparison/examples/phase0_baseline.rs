//! Phase 0 baseline: Eva R1CS stats + NeutronNova smoke test on BN254.
//!
//! This example does **not** compare prove time fairly yet — the NeutronNova circuit is a
//! placeholder. It validates infrastructure and prints both sides for manual recording.
//!
//! # Usage
//!
//! ```bash
//! NUM_STEPS=4 CIRCUIT_DEGREE=1024 cargo run --release -p comparison --example phase0_baseline
//! ```

use comparison::{
    eva_step::{blocks_per_step_from_env, synthesize_both_r1cs},
    neutronnova::{run_neutronnova_smoke, PlaceholderConfig},
    print_r1cs_stats, stats_from_r1cs,
};

fn main() {
    println!("=== Phase 0 baseline ===\n");

    // --- Part 1: Eva R1CS characterization ---
    let blocks_per_step = blocks_per_step_from_env();
    println!("[1/2] Eva R1CS stats (BLOCKS_PER_STEP={blocks_per_step})");

    let (step_r1cs, augmented_r1cs) = synthesize_both_r1cs(blocks_per_step)
        .unwrap_or_else(|e| panic!("Eva synthesis failed: {e}"));

    let eva_stats = [
        stats_from_r1cs("eva-step-only", &step_r1cs),
        stats_from_r1cs("eva-augmented", &augmented_r1cs),
    ];
    print_r1cs_stats(&eva_stats);

    // --- Part 2: NeutronNova smoke test ---
    let nn_config = PlaceholderConfig::from_env();
    println!();
    println!(
        "[2/2] NeutronNova smoke test (NUM_STEPS={}, CIRCUIT_DEGREE={})",
        nn_config.num_steps, nn_config.circuit_degree
    );
    println!("      Placeholder squaring circuit on BN254 — NOT Eva's logic yet.\n");

    let timings = run_neutronnova_smoke(&nn_config)
        .unwrap_or_else(|e| panic!("NeutronNova smoke test failed: {e}"));

    println!("  {timings}");
    println!();
    println!("=== Phase 0 complete ===");
    println!("Next steps:");
    println!("  1. Copy output to docs/spartan2-comparison/results/phase0.md");
    println!("  2. Phase 1: size PlaceholderStepCircuit.degree ≈ eva-augmented constraints");
    println!("  3. Add Nova prove_step timing harness");
    println!();
    println!("See docs/spartan2-comparison/PHASES.md for the full plan.");
}
