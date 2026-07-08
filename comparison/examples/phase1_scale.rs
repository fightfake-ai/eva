//! Multi-scale Phase 1 benchmark: sweep BLOCKS_PER_STEP and compare Nova vs NeutronNova.
//!
//! # Usage
//!
//! ```bash
//! # Default scales: 4, 16, 64 (matched constraint counts)
//! cargo run --release -p comparison --example phase1_scale
//!
//! # Include full Eva scale (Nova only — NeutronNova at 1.4M constraints may OOM)
//! SCALES=4,16,64,256 SKIP_NN_AT=256 cargo run --release -p comparison --example phase1_scale
//! ```
//!
//! Environment:
//! - `SCALES` — comma-separated BLOCKS_PER_STEP values (default `4,16,64`)
//! - `SKIP_NN_AT` — comma-separated scales at which to skip NeutronNova (default `256`)
//! - `MATCH_EVA=1` — match placeholder degree to Eva constraints (recommended)

use comparison::phase1::{format_scale_row, run_phase1_comparison, Phase1Config};

fn parse_scales() -> Vec<usize> {
    std::env::var("SCALES")
        .unwrap_or_else(|_| "4,16,64".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

fn skip_nn_at() -> Vec<usize> {
    std::env::var("SKIP_NN_AT")
        .unwrap_or_else(|_| "256".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

fn main() {
    let scales = parse_scales();
    let skip_at: Vec<usize> = skip_nn_at();
    let match_eva = std::env::var("MATCH_EVA")
        .map(|s| s != "0" && !s.eq_ignore_ascii_case("false"))
        .unwrap_or(true);

    println!("=== Phase 1 scaling study ===");
    println!("SCALES={:?} MATCH_EVA={} SKIP_NN_AT={:?}\n", scales, match_eva, skip_at);

    println!("| blocks | constraints | nova_prove_ms | nova_cmT_ms | nova_preprocess_ms | nn_prove/step_ms | nn/nova |");
    println!("|--------|-------------|---------------|-------------|--------------------|--------------------|---------|");

    for &blocks in &scales {
        eprintln!("--- Running BLOCKS_PER_STEP={blocks} ---");
        let mut config = Phase1Config::with_blocks(blocks, match_eva);
        config.skip_neutronnova = skip_at.contains(&blocks);

        let result = run_phase1_comparison(&config)
            .unwrap_or_else(|e| panic!("scale {blocks} failed: {e}"));
        println!("{}", format_scale_row(&result));
    }

    println!();
    println!("Copy table to docs/spartan2-comparison/results/phase1.md");
}
