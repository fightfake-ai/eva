//! Phase 1 benchmark: Nova vs NeutronNova at matched (or configured) R1CS scale.
//!
//! # Usage
//!
//! ```bash
//! # Quick iteration (small Eva circuit + small placeholder)
//! QUICK=1 cargo run --release -p comparison --example phase1_benchmark
//!
//! # Full Eva scale (slow — may take tens of minutes and significant RAM)
//! BLOCKS_PER_STEP=256 MATCH_EVA=1 NUM_STEPS=1 cargo run --release -p comparison --example phase1_benchmark
//! ```
//!
//! # Environment
//!
//! | Variable | Default | Meaning |
//! |----------|---------|---------|
//! | `QUICK` | unset | If set, uses `BLOCKS_PER_STEP=4` and `CIRCUIT_DEGREE=1000` |
//! | `BLOCKS_PER_STEP` | `256` | Eva macroblocks per Nova step |
//! | `MATCH_EVA` | unset | If set, sets `CIRCUIT_DEGREE` ≈ Eva augmented constraint count |
//! | `NUM_STEPS` | `1` | NeutronNova batch size (prove time amortized per step in output) |
//! | `CIRCUIT_DEGREE` | `10000` (quick) / from MATCH_EVA | Placeholder squaring constraints |
//! | `IS_SMALL` | `false` | Spartan2 small-integer fast path |

use comparison::{
    degree_for_target_constraints, run_neutronnova_benchmark, run_nova_baseline,
    NovaBaselineConfig, PlaceholderConfig,
};

fn main() {
    let quick = std::env::var("QUICK").is_ok();

    let nova_config = if quick {
        comparison::nova_baseline::quick_nova_config()
    } else {
        NovaBaselineConfig::from_env()
    };

    println!("=== Phase 1 benchmark ===\n");
    println!("[1/2] Nova baseline (Eva augmented circuit, CPU backend)");
    println!("      BLOCKS_PER_STEP={}\n", nova_config.blocks_per_step);

    let nova = run_nova_baseline(&nova_config).unwrap_or_else(|e| panic!("Nova failed: {e}"));
    println!("  {nova}\n");

    let target_constraints = if std::env::var("MATCH_EVA").is_ok() {
        nova.num_constraints
    } else if quick {
        1000
    } else {
        std::env::var("CIRCUIT_DEGREE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000)
    };

    let circuit_degree = degree_for_target_constraints(target_constraints);

    let nn_config = PlaceholderConfig {
        num_steps: std::env::var("NUM_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2),
        circuit_degree,
        is_small: std::env::var("IS_SMALL")
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    };

    println!("[2/2] NeutronNova baseline (BN254 placeholder, NOT Eva logic)");
    println!(
        "      NUM_STEPS={} CIRCUIT_DEGREE={} (target constraints ≈ {target_constraints})\n",
        nn_config.num_steps, nn_config.circuit_degree
    );

    let nn = run_neutronnova_benchmark(&nn_config)
        .unwrap_or_else(|e| panic!("NeutronNova failed: {e}"));
    println!("  {nn}\n");

    let nn_per_step = nn.prove_ms / nn_config.num_steps as f64;

    println!("=== Summary ===");
    println!("  Nova prove_step (1 IVC step):     {:>10.1} ms  ({} constraints)",
        nova.prove_step_ms, nova.num_constraints);
    println!("  NeutronNova prove (batch):        {:>10.1} ms  ({} constraints x {} steps)",
        nn.prove_ms, nn.num_constraints, nn.num_steps);
    println!("  NeutronNova prove (per step):     {:>10.1} ms",
        nn_per_step);
    println!();
    println!("  Nova compute_cmT only:            {:>10.1} ms", nova.compute_cmT_ms);
    println!("  Nova preprocess (incl. synthesis):{:>10.1} ms", nova.preprocess_ms);
    println!();
    if nova.num_constraints != nn.num_constraints {
        println!("  NOTE: constraint counts differ — set MATCH_EVA=1 for matched placeholder degree.");
        println!("        This is NOT an apples-to-apples logic comparison (placeholder vs Eva).");
    }
    println!();
    println!("Record results in docs/spartan2-comparison/results/phase1.md");
}
