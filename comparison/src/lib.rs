//! Nova vs Spartan2/NeutronNova comparison utilities for Eva.
//!
//! See `docs/spartan2-comparison/` for the full study design.

pub mod eva_step;
pub mod neutronnova;
pub mod r1cs_stats;

pub use eva_step::{synthesize_augmented_step_r1cs, synthesize_step_only_r1cs, EvaCircuitKind};
pub use neutronnova::{run_neutronnova_smoke, NeutronNovaTimings, PlaceholderConfig};
pub use r1cs_stats::{R1csStats, print_r1cs_stats, stats_from_r1cs};
