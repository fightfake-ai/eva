//! Nova vs Spartan2/NeutronNova comparison utilities for Eva.
//!
//! See `docs/spartan2-comparison/` for the full study design.

pub mod eva_step;
pub mod neutronnova;
pub mod nova_baseline;
pub mod phase1;
pub mod r1cs_stats;

pub use eva_step::{synthesize_augmented_step_r1cs, synthesize_step_only_r1cs, EvaCircuitKind};
pub use neutronnova::{
    count_placeholder_constraints, degree_for_target_constraints, run_neutronnova_benchmark,
    run_neutronnova_smoke, NeutronNovaTimings, PlaceholderConfig, PlaceholderStepCircuit,
};
pub use nova_baseline::{run_nova_baseline, NovaBaselineConfig, NovaBaselineTimings};
pub use phase1::{format_scale_row, run_phase1_comparison, Phase1Config, Phase1Result};
pub use r1cs_stats::{R1csStats, print_r1cs_stats, stats_from_r1cs};
