//! LogUp as a Spartan2 `SpartanCircuit` for NeutronNova prove/verify.
//!
//! # Witness split (Phase 2.0)
//!
//! | Phase | Contents |
//! |-------|----------|
//! | `shared` | empty (table is constants `0..255`) |
//! | `precommitted` | empty (BN254 NeutronNova verify is fragile here — see Phase 0) |
//! | `synthesize` | queries + histo + fixed challenge `c` + LogUp R1CS + public IO |
//!
//! ## Challenge binding (deferred)
//!
//! Eva derives `c = Poseidon(cmQ)` after committing queries+histo. Target end-state:
//! queries/histo in `precommitted`, `num_challenges = 1` for transcript `c`, inverses
//! in `synthesize`. Phase 2.0 uses a fixed witness `c` and all-in-`synthesize` layout
//! so NeutronNova setup→prove→verify passes on BN254 while we validate LogUp constraints.

use std::marker::PhantomData;
use std::time::Instant;

use bellpepper_core::{num::AllocatedNum, ConstraintSystem, SynthesisError};
use ff::Field;
use spartan2::{
    neutronnova_zk::NeutronNovaZkSNARK,
    provider::Bn254Engine,
    traits::{circuit::SpartanCircuit, Engine},
};

use super::lookup::{
    build_histogram, enforce_logup, expected_logup_constraints, TABLE_SIZE,
};

type E = Bn254Engine;

/// YUV 4:2:0 macroblock pixel count: 16×16 Y + 8×8 U + 8×8 V.
pub const PIXELS_PER_MB: usize = 16 * 16 + 8 * 8 + 8 * 8; // 384

/// Config for LogUp NeutronNova smoke / benchmark runs.
#[derive(Clone, Debug)]
pub struct LogUpConfig {
    pub num_steps: usize,
    /// Number of lookup queries (must be ∈ table). Use [`PIXELS_PER_MB`] for 1-MB pixels.
    pub num_queries: usize,
    pub is_small: bool,
}

impl LogUpConfig {
    /// Load from env: `NUM_STEPS` (default 2), `NUM_QUERIES` (default [`PIXELS_PER_MB`]),
    /// `IS_SMALL` (default false — inverses are full field elements).
    pub fn from_env() -> Self {
        Self {
            num_steps: std::env::var("NUM_STEPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2),
            num_queries: std::env::var("NUM_QUERIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(PIXELS_PER_MB),
            is_small: std::env::var("IS_SMALL")
                .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }
}

/// Timing breakdown for one LogUp NeutronNova run.
#[derive(Clone, Debug, Default)]
pub struct LogUpTimings {
    pub setup_ms: f64,
    pub prep_prove_ms: f64,
    pub prove_ms: f64,
    pub verify_ms: f64,
    pub num_steps: usize,
    pub num_queries: usize,
    pub num_constraints: usize,
    pub expected_logup_constraints: usize,
    pub is_small: bool,
}

impl std::fmt::Display for LogUpTimings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LogUp+NeutronNova(BN254): steps={} queries={} constraints={} (logup_core≈{}) is_small={} setup={:.1}ms prep={:.1}ms prove={:.1}ms verify={:.1}ms",
            self.num_steps,
            self.num_queries,
            self.num_constraints,
            self.expected_logup_constraints,
            self.is_small,
            self.setup_ms,
            self.prep_prove_ms,
            self.prove_ms,
            self.verify_ms,
        )
    }
}

/// Default public LogUp challenge (must not collide with table `0..255`).
pub const DEFAULT_LOGUP_CHALLENGE: u64 = 1_000_003;

/// Bellpepper LogUp circuit: table `0..255`, arbitrary query list (each byte).
///
/// Not a full Eva step — encode / Griffin / AugmentedFCircuit are deferred.
#[derive(Clone, Debug)]
pub struct LogUpCircuit {
    /// Query values in `0..255` (pixels and/or encode chunks).
    pub queries: Vec<u8>,
    /// Public LogUp challenge `c` (fixed for Phase 2.0; see module docs).
    pub challenge: <E as Engine>::Scalar,
    _p: PhantomData<E>,
}

impl LogUpCircuit {
    pub fn new(queries: Vec<u8>) -> Self {
        for &q in &queries {
            assert!(
                (q as usize) < TABLE_SIZE,
                "query {q} outside table 0..{TABLE_SIZE}"
            );
        }
        Self {
            queries,
            challenge: <E as Engine>::Scalar::from(DEFAULT_LOGUP_CHALLENGE),
            _p: PhantomData,
        }
    }

    /// Synthetic queries for size `n` (cycling `0..255`).
    pub fn synthetic(n: usize) -> Self {
        Self::new((0..n).map(|i| (i % TABLE_SIZE) as u8).collect())
    }

    /// 1 macroblock of synthetic pixel queries (384 values).
    pub fn one_mb_pixels() -> Self {
        Self::synthetic(PIXELS_PER_MB)
    }
}

impl SpartanCircuit<E> for LogUpCircuit {
    fn public_values(&self) -> Result<Vec<<E as Engine>::Scalar>, SynthesisError> {
        // Match Phase 0 / Spartan2 SHA-256 NeutronNova examples.
        Ok(vec![<E as Engine>::Scalar::ZERO])
    }

    fn shared<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        _cs: &mut CS,
    ) -> Result<Vec<AllocatedNum<<E as Engine>::Scalar>>, SynthesisError> {
        Ok(vec![])
    }

    fn precommitted<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        _cs: &mut CS,
        _shared: &[AllocatedNum<<E as Engine>::Scalar>],
    ) -> Result<Vec<AllocatedNum<<E as Engine>::Scalar>>, SynthesisError> {
        // Phase 0 note: on BN254, constraints/witness only in `precommitted` caused
        // NeutronNova verify failures. Keep queries in `synthesize` for now (same as
        // PlaceholderStepCircuit). Moving queries back to precommitted is a follow-up
        // once FS challenge binding works.
        Ok(vec![])
    }

    fn num_challenges(&self) -> usize {
        // See module docs: FS challenges deferred for NeutronNova bring-up.
        0
    }

    fn synthesize<CS: ConstraintSystem<<E as Engine>::Scalar>>(
        &self,
        cs: &mut CS,
        _shared: &[AllocatedNum<<E as Engine>::Scalar>],
        _precommitted: &[AllocatedNum<<E as Engine>::Scalar>],
        _challenges: Option<&[<E as Engine>::Scalar]>,
    ) -> Result<(), SynthesisError> {
        let mut queries = Vec::with_capacity(self.queries.len());
        for (i, &q) in self.queries.iter().enumerate() {
            let v = AllocatedNum::alloc(cs.namespace(|| format!("q_{i}")), || {
                Ok(<E as Engine>::Scalar::from(q as u64))
            })?;
            queries.push(v);
        }

        let histo_vals = build_histogram(&self.queries, TABLE_SIZE);
        let mut histo = Vec::with_capacity(TABLE_SIZE);
        for (i, &count) in histo_vals.iter().enumerate() {
            let e = AllocatedNum::alloc(cs.namespace(|| format!("e_{i}")), || {
                Ok(<E as Engine>::Scalar::from(count))
            })?;
            histo.push(e);
        }

        let challenge = AllocatedNum::alloc(cs.namespace(|| "logup_challenge"), || {
            Ok(self.challenge)
        })?;

        enforce_logup(
            &mut cs.namespace(|| "logup"),
            &challenge,
            &queries,
            &histo,
            TABLE_SIZE,
        )?;

        let pub0 = AllocatedNum::alloc(cs.namespace(|| "pub0"), || {
            Ok(<E as Engine>::Scalar::ZERO)
        })?;
        pub0.inputize(cs.namespace(|| "inputize pub0"))?;
        Ok(())
    }
}

/// Count R1CS constraints for a LogUp circuit (ShapeCS, no prove).
pub fn count_logup_constraints(num_queries: usize) -> Result<usize, String> {
    use spartan2::bellpepper::shape_cs::ShapeCS;

    let circuit = LogUpCircuit::synthetic(num_queries);
    let mut cs = ShapeCS::<E>::new();
    let shared = circuit
        .shared(&mut cs)
        .map_err(|e| format!("shared: {e}"))?;
    let precommitted = circuit
        .precommitted(&mut cs, &shared)
        .map_err(|e| format!("precommitted: {e}"))?;
    circuit
        .synthesize(&mut cs, &shared, &precommitted, None)
        .map_err(|e| format!("synthesize: {e}"))?;
    Ok(cs.num_constraints())
}

/// Run NeutronNova setup → prep_prove → prove → verify on a LogUp circuit batch.
pub fn run_logup_neutronnova(config: &LogUpConfig) -> Result<LogUpTimings, String> {
    let num_constraints = count_logup_constraints(config.num_queries)?;
    let expected = expected_logup_constraints(config.num_queries, TABLE_SIZE);

    let proto = LogUpCircuit::synthetic(config.num_queries);

    let t0 = Instant::now();
    let (pk, vk) = NeutronNovaZkSNARK::<E>::setup(&proto, &proto, config.num_steps)
        .map_err(|e| format!("NeutronNova setup failed: {e}"))?;
    let setup_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Distinct query seeds per step so the batch is non-trivial.
    let step_circuits: Vec<LogUpCircuit> = (0..config.num_steps)
        .map(|s| {
            LogUpCircuit::new(
                (0..config.num_queries)
                    .map(|i| ((i + s * 17) % TABLE_SIZE) as u8)
                    .collect(),
            )
        })
        .collect();
    let core_circuit = step_circuits[0].clone();

    let t1 = Instant::now();
    let prep = NeutronNovaZkSNARK::<E>::prep_prove(
        &pk,
        &step_circuits,
        &core_circuit,
        config.is_small,
    )
    .map_err(|e| format!("NeutronNova prep_prove failed: {e}"))?;
    let prep_prove_ms = t1.elapsed().as_secs_f64() * 1000.0;

    let t2 = Instant::now();
    let (proof, _prep) = NeutronNovaZkSNARK::<E>::prove(
        &pk,
        &step_circuits,
        &core_circuit,
        prep,
        config.is_small,
    )
    .map_err(|e| format!("NeutronNova prove failed: {e}"))?;
    let prove_ms = t2.elapsed().as_secs_f64() * 1000.0;

    let t3 = Instant::now();
    proof
        .verify(&vk, config.num_steps)
        .map_err(|e| format!("NeutronNova verify failed: {e}"))?;
    let verify_ms = t3.elapsed().as_secs_f64() * 1000.0;

    Ok(LogUpTimings {
        setup_ms,
        prep_prove_ms,
        prove_ms,
        verify_ms,
        num_steps: config.num_steps,
        num_queries: config.num_queries,
        num_constraints,
        expected_logup_constraints: expected,
        is_small: config.is_small,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_count_matches_formula_plus_io() {
        // LogUp core = Q+T+1; plus inputize of pub0 typically adds 1 constraint.
        let q = 32;
        let n = count_logup_constraints(q).expect("shape");
        let core = expected_logup_constraints(q, TABLE_SIZE);
        assert!(
            n >= core && n <= core + 4,
            "constraints {n} not near core {core}"
        );
    }

    #[test]
    fn neutronnova_logup_smoke_small() {
        let config = LogUpConfig {
            num_steps: 2,
            num_queries: 16,
            is_small: false,
        };
        run_logup_neutronnova(&config).expect("LogUp NeutronNova smoke");
    }
}
