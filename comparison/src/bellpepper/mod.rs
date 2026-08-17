//! Phase 2: Eva LogUp lookup argument ported to bellpepper for Spartan2 / NeutronNova.
//!
//! See `docs/spartan2-comparison/LOOKUPS.md` for witness layout and port plan.

pub mod lookup;
pub mod relaxed_check;
pub mod step;

pub use lookup::{build_histogram, expected_logup_constraints, TABLE_SIZE};
pub use step::{
    count_logup_constraints, run_logup_neutronnova, LogUpCircuit, LogUpConfig, LogUpTimings,
    PIXELS_PER_MB,
};
