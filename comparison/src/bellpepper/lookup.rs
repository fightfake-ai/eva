//! Bellpepper LogUp gadgets matching Eva's `LookupArgument` (ark-r1cs-std).
//!
//! Reference: `folding-schemes/src/frontend/mod.rs`
//!
//! # Identity
//!
//! For table `T[0..T)`, queries `q[0..Q)`, multiplicities `e[0..T)`, challenge `c`:
//!
//! ```text
//! Σ_i  e_i / (c - T_i)  =  Σ_j  1 / (c - q_j)
//! ```
//!
//! Enforced as R1CS:
//! - LHS (T constraints): `(c - T_i) · inv_i = e_i`
//! - RHS (Q constraints): `(c - q_j) · inv_j = 1`
//! - Identity (1 constraint): `(Σ inv_i) · 1 = (Σ inv_j)`
//!
//! Total: **Q + T + 1** constraints (plus any public-IO / challenge allocation overhead).

use bellpepper_core::{num::AllocatedNum, ConstraintSystem, SynthesisError};
use ff::PrimeField;

/// Eva table size for `MB_BITS = 8` (`0..255`).
pub const TABLE_SIZE: usize = 256;

/// Expected LogUp R1CS constraint count for `Q` queries and table size `T`
/// (excluding public-IO / challenge allocation overhead).
pub fn expected_logup_constraints(num_queries: usize, table_size: usize) -> usize {
    num_queries + table_size + 1
}

/// Build histogram multiplicities `e_i = |{ j : q_j = i }|` for table `0..table_size`.
///
/// Panics if any query is outside `0..table_size` (mirrors Eva's `.unwrap()` on miss).
pub fn build_histogram(queries: &[u8], table_size: usize) -> Vec<u64> {
    let mut histo = vec![0u64; table_size];
    for &q in queries {
        let idx = q as usize;
        assert!(
            idx < table_size,
            "query value {q} outside table 0..{table_size}"
        );
        histo[idx] += 1;
    }
    histo
}

/// Allocate LogUp aux witnesses (inverses) and enforce the LogUp identity.
///
/// `queries` and `histo` must already be allocated (typically in `precommitted`).
/// `challenge` is the LogUp challenge `c` (typically an `alloc_input` from NeutronNova).
///
/// Returns `(lhs_invs, rhs_invs)`.
pub fn enforce_logup<Scalar, CS>(
    cs: &mut CS,
    challenge: &AllocatedNum<Scalar>,
    queries: &[AllocatedNum<Scalar>],
    histo: &[AllocatedNum<Scalar>],
    table_size: usize,
) -> Result<(Vec<AllocatedNum<Scalar>>, Vec<AllocatedNum<Scalar>>), SynthesisError>
where
    Scalar: PrimeField,
    CS: ConstraintSystem<Scalar>,
{
    assert_eq!(
        histo.len(),
        table_size,
        "histo length {} != table_size {table_size}",
        histo.len()
    );

    let c_val = challenge.get_value();

    // --- LHS: (c - T_i) · inv_i = e_i ---
    let mut lhs_invs = Vec::with_capacity(table_size);
    for i in 0..table_size {
        let t_i = Scalar::from(i as u64);
        let e = &histo[i];
        let e_val = e.get_value();

        let inv = AllocatedNum::alloc(cs.namespace(|| format!("lhs_inv_{i}")), || {
            let c = c_val.ok_or(SynthesisError::AssignmentMissing)?;
            let e_v = e_val.ok_or(SynthesisError::AssignmentMissing)?;
            let denom = c - t_i;
            let inv_denom = denom.invert().unwrap_or(Scalar::ZERO);
            Ok(e_v * inv_denom)
        })?;

        // (c - T_i) * inv = e
        // LinearCombination form: A = c - T_i, B = inv, C = e
        cs.enforce(
            || format!("lhs_{i}"),
            |lc| lc + challenge.get_variable() - (t_i, CS::one()),
            |lc| lc + inv.get_variable(),
            |lc| lc + e.get_variable(),
        );

        lhs_invs.push(inv);
    }

    // --- RHS: (c - q_j) · inv_j = 1 ---
    let mut rhs_invs = Vec::with_capacity(queries.len());
    for (j, q) in queries.iter().enumerate() {
        let q_val = q.get_value();

        let inv = AllocatedNum::alloc(cs.namespace(|| format!("rhs_inv_{j}")), || {
            let c = c_val.ok_or(SynthesisError::AssignmentMissing)?;
            let q_v = q_val.ok_or(SynthesisError::AssignmentMissing)?;
            let denom = c - q_v;
            Ok(denom.invert().unwrap_or(Scalar::ZERO))
        })?;

        cs.enforce(
            || format!("rhs_{j}"),
            |lc| lc + challenge.get_variable() - q.get_variable(),
            |lc| lc + inv.get_variable(),
            |lc| lc + CS::one(),
        );

        rhs_invs.push(inv);
    }

    // --- Identity: Σ lhs_inv = Σ rhs_inv ---
    cs.enforce(
        || "logup_identity",
        |lc| {
            let mut acc = lc;
            for inv in &lhs_invs {
                acc = acc + inv.get_variable();
            }
            acc
        },
        |lc| lc + CS::one(),
        |lc| {
            let mut acc = lc;
            for inv in &rhs_invs {
                acc = acc + inv.get_variable();
            }
            acc
        },
    );

    Ok((lhs_invs, rhs_invs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_counts_match_queries() {
        let queries = [0u8, 1, 1, 255, 0, 0];
        let h = build_histogram(&queries, TABLE_SIZE);
        assert_eq!(h[0], 3);
        assert_eq!(h[1], 2);
        assert_eq!(h[255], 1);
        assert_eq!(h.iter().sum::<u64>(), queries.len() as u64);
    }

    #[test]
    fn expected_constraints_formula() {
        // 1-MB pixel-only: 384 queries + 256 table + 1 identity
        assert_eq!(expected_logup_constraints(384, TABLE_SIZE), 641);
        // Full 1-MB NoOp encode queries (Phase 0 scale / 256 ≈ 2320)
        assert_eq!(expected_logup_constraints(2320, TABLE_SIZE), 2577);
    }
}
