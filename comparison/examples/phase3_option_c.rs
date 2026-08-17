//! Option C dual-path: Nova IVC + Groth16 vs transparent Spartan (full DeciderEth).
//!
//! ```bash
//! # Full transparent Spartan only (recommended first run; Groth16 setup ~2 min)
//! SKIP_G16=1 SPARTAN_MODE=full cargo run --release -p comparison --example phase3_option_c
//!
//! # Both Groth16 + full Spartan
//! cargo run --release -p comparison --example phase3_option_c
//!
//! # Legacy primary-only Spartan (incomplete statement)
//! SKIP_G16=1 SPARTAN_MODE=primary cargo run --release -p comparison --example phase3_option_c
//!
//! SKIP_SPARTAN=1
//! BLOCKS_PER_STEP=4 NUM_STEPS=4
//! ```

use comparison::{run_option_c, OptionCConfig};

fn main() {
    let config = OptionCConfig::from_env();
    println!("=== Phase 3 / Option C: Groth16 vs transparent Spartan ===");
    println!(
        "BLOCKS_PER_STEP={} NUM_STEPS={} run_g16={} run_spartan={} spartan_mode={:?}",
        config.blocks_per_step,
        config.num_steps,
        config.run_groth16,
        config.run_spartan,
        config.spartan_mode
    );
    println!("Workload: EditOnly + Brightness (synthetic non-zero blocks)");
    match config.spartan_mode {
        comparison::option_c::SpartanMode::Full => {
            println!(
                "Spartan FULL: proves DeciderEthCircuit R1CS (primary + CycleFold + sigma), transparent.\n"
            );
        }
        comparison::option_c::SpartanMode::Primary => {
            println!(
                "Spartan PRIMARY: proves folded primary relaxed R1CS only (no CycleFold/sigma).\n"
            );
        }
    }

    match run_option_c(&config) {
        Ok(r) => {
            println!(
                "Nova: preprocess={:.1}ms ivc={:.1}ms primary_constraints={}",
                r.nova_preprocess_ms, r.nova_ivc_ms, r.primary_r1cs_constraints
            );
            if let Some(g) = &r.groth16 {
                println!(
                    "Groth16: setup={:.1}ms prove={:.1}ms verify={:.1}ms proof={}B ({})",
                    g.setup_ms, g.prove_ms, g.verify_ms, g.proof_bytes, g.notes
                );
            }
            if let Some(s) = &r.spartan {
                println!(
                    "Spartan: setup={:.1}ms prove={:.1}ms verify={:.1}ms proof={}B ({})",
                    s.setup_ms, s.prove_ms, s.verify_ms, s.proof_bytes, s.notes
                );
            }
            println!("\nSee docs/spartan2-comparison/OPTION_C_TRANSPARENT.md");
        }
        Err(e) => {
            eprintln!("Option C failed: {e}");
            std::process::exit(1);
        }
    }
}
