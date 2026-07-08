//! Phase 1 benchmark: Nova vs NeutronNova at matched (or configured) R1CS scale.

use comparison::phase1::{run_phase1_comparison, Phase1Config};

fn main() {
    let config = if std::env::var("QUICK").is_ok() {
        let mut c = Phase1Config::quick();
        if std::env::var("MATCH_EVA").is_ok() {
            c.match_eva = true;
            c.circuit_degree = None;
        }
        c
    } else {
        Phase1Config::from_env()
    };

    println!("=== Phase 1 benchmark ===\n");
    println!(
        "[config] BLOCKS_PER_STEP={} MATCH_EVA={} NUM_STEPS={} SKIP_NN={}\n",
        config.blocks_per_step,
        config.match_eva,
        config.num_steps,
        config.skip_neutronnova,
    );

    let result = run_phase1_comparison(&config).unwrap_or_else(|e| panic!("Phase 1 failed: {e}"));

    if let Some(nn) = &result.neutronnova {
        println!("  {}\n", result.nova);
        println!("  {nn}\n");
    } else {
        println!("  {}\n", result.nova);
    }

    result.print_summary();
    println!("Record results in docs/spartan2-comparison/results/phase1.md");
}
