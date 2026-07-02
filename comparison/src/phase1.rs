//! Phase 1 paired benchmark: Nova vs NeutronNova at a given Eva scale.
//!
//! Used by `phase1_benchmark` and `phase1_scale` examples.

use crate::eva_step::blocks_per_step_from_env;
use crate::neutronnova::{
    degree_for_target_constraints, run_neutronnova_benchmark, PlaceholderConfig,
};
use crate::nova_baseline::{run_nova_baseline, NovaBaselineConfig, NovaBaselineTimings};
use crate::neutronnova::NeutronNovaTimings;

/// Run both Nova and NeutronNova baselines at the same Eva scale.
#[derive(Clone, Debug)]
pub struct Phase1Config {
    pub blocks_per_step: usize,
    /// When true, set NeutronNova placeholder degree ≈ Eva augmented constraint count.
    pub match_eva: bool,
    pub num_steps: usize,
    pub is_small: bool,
    /// Fixed placeholder degree when `match_eva` is false.
    pub circuit_degree: Option<usize>,
    /// Skip NeutronNova (useful for large scales that may OOM).
    pub skip_neutronnova: bool,
}

impl Phase1Config {
    pub fn from_env() -> Self {
        Self {
            blocks_per_step: blocks_per_step_from_env(),
            match_eva: std::env::var("MATCH_EVA").is_ok(),
            num_steps: std::env::var("NUM_STEPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2),
            is_small: std::env::var("IS_SMALL")
                .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            circuit_degree: std::env::var("CIRCUIT_DEGREE")
                .ok()
                .and_then(|s| s.parse().ok()),
            skip_neutronnova: std::env::var("SKIP_NN").is_ok(),
        }
    }

    pub fn quick() -> Self {
        Self {
            blocks_per_step: 4,
            match_eva: false,
            num_steps: 2,
            is_small: false,
            circuit_degree: Some(1000),
            skip_neutronnova: false,
        }
    }

    pub fn with_blocks(blocks_per_step: usize, match_eva: bool) -> Self {
        Self {
            blocks_per_step,
            match_eva,
            num_steps: 2,
            is_small: false,
            circuit_degree: None,
            skip_neutronnova: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Phase1Result {
    pub nova: NovaBaselineTimings,
    pub neutronnova: Option<NeutronNovaTimings>,
}

impl Phase1Result {
    pub fn nn_prove_per_step_ms(&self) -> Option<f64> {
        self.neutronnova.as_ref().map(|nn| {
            nn.prove_ms / nn.num_steps.max(1) as f64
        })
    }

    pub fn print_summary(&self) {
        let nova = &self.nova;
        println!("=== Summary (BLOCKS_PER_STEP={}) ===", nova.blocks_per_step);
        println!(
            "  Nova prove_step:                  {:>10.1} ms  ({} constraints)",
            nova.prove_step_ms, nova.num_constraints
        );
        if let Some(nn) = &self.neutronnova {
            let per_step = nn.prove_ms / nn.num_steps.max(1) as f64;
            println!(
                "  NeutronNova prove (batch):        {:>10.1} ms  ({} constraints x {} steps)",
                nn.prove_ms, nn.num_constraints, nn.num_steps
            );
            println!("  NeutronNova prove (per step):     {:>10.1} ms", per_step);
            if nova.num_constraints == nn.num_constraints {
                let ratio = per_step / nova.prove_step_ms.max(0.001);
                println!("  Ratio NN/Nova prove per step:     {:>10.2}x", ratio);
            }
        } else {
            println!("  NeutronNova: skipped (SKIP_NN=1)");
        }
        println!();
        println!(
            "  Nova compute_cmT:                 {:>10.1} ms",
            nova.compute_cmT_ms
        );
        println!(
            "  Nova preprocess:                  {:>10.1} ms",
            nova.preprocess_ms
        );
    }
}

/// Run Phase 1 comparison for the given config.
pub fn run_phase1_comparison(config: &Phase1Config) -> Result<Phase1Result, String> {
    let nova = run_nova_baseline(&NovaBaselineConfig {
        blocks_per_step: config.blocks_per_step,
    })?;

    let neutronnova = if config.skip_neutronnova {
        None
    } else {
        let target = if config.match_eva {
            nova.num_constraints
        } else {
            config.circuit_degree.unwrap_or(10_000)
        };
        let nn_config = PlaceholderConfig {
            num_steps: config.num_steps,
            circuit_degree: degree_for_target_constraints(target),
            is_small: config.is_small,
        };
        Some(run_neutronnova_benchmark(&nn_config)?)
    };

    Ok(Phase1Result {
        nova,
        neutronnova,
    })
}

/// Markdown table row for scaling results.
pub fn format_scale_row(result: &Phase1Result) -> String {
    let nova = &result.nova;
    let nn_ms = result
        .nn_prove_per_step_ms()
        .map(|ms| format!("{ms:.1}"))
        .unwrap_or_else(|| "—".into());
    let ratio = match (&result.neutronnova, nova.prove_step_ms) {
        (Some(_), t) if t > 0.0 => {
            format!("{:.2}", result.nn_prove_per_step_ms().unwrap() / t)
        }
        _ => "—".into(),
    };
    format!(
        "| {} | {} | {:.1} | {:.1} | {:.1} | {} | {} |",
        nova.blocks_per_step,
        nova.num_constraints,
        nova.prove_step_ms,
        nova.compute_cmT_ms,
        nova.preprocess_ms,
        nn_ms,
        ratio,
    )
}
