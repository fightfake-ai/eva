//! Phase 2 smoke: Eva LogUp (bellpepper) under NeutronNova on BN254.
//!
//! Proves a **lookup-only** circuit (not full Eva encode/edit). Default query count is
//! one macroblock of pixels (384). Override with `NUM_QUERIES`.
//!
//! ```bash
//! # 1-MB pixels (default)
//! cargo run --release -p comparison --example phase2_lookup_smoke
//!
//! # Tiny iteration
//! NUM_QUERIES=16 NUM_STEPS=2 cargo run --release -p comparison --example phase2_lookup_smoke
//!
//! # Full 1-MB NoOp query scale (~2320 committed queries before histo)
//! NUM_QUERIES=2320 cargo run --release -p comparison --example phase2_lookup_smoke
//! ```
//!
//! Record output in `docs/spartan2-comparison/results/phase2.md`.

use comparison::{count_logup_constraints, run_logup_neutronnova, LogUpConfig, TABLE_SIZE};

fn main() {
    let config = LogUpConfig::from_env();
    println!("=== Phase 2: LogUp + NeutronNova smoke ===");
    println!(
        "NUM_STEPS={} NUM_QUERIES={} TABLE_SIZE={} IS_SMALL={}",
        config.num_steps, config.num_queries, TABLE_SIZE, config.is_small
    );

    match count_logup_constraints(config.num_queries) {
        Ok(n) => println!("ShapeCS constraints: {n}"),
        Err(e) => {
            eprintln!("constraint count failed: {e}");
            std::process::exit(1);
        }
    }

    match run_logup_neutronnova(&config) {
        Ok(t) => {
            println!("{t}");
            println!("verify: OK");
            println!("Record results in docs/spartan2-comparison/results/phase2.md");
        }
        Err(e) => {
            eprintln!("LogUp NeutronNova failed: {e}");
            std::process::exit(1);
        }
    }
}
