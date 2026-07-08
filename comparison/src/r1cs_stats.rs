//! R1CS dimension statistics extracted from Eva's arkworks constraint systems.
//!
//! These numbers drive Phase 1 placeholder circuit sizing: we need to know Eva's
//! real constraint / variable / sparsity profile before building a matched bellpepper circuit.

use std::fmt;

use folding_schemes::ccs::r1cs::R1CS;
use folding_schemes::utils::vec::SparseMatrix;

/// Summary statistics for one R1CS instance (A,B,C over the same column count).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R1csStats {
    /// Human-readable label (e.g. `"eva-step-only"`, `"eva-augmented"`).
    pub label: String,
    /// Number of constraint rows (= `A.n_rows`).
    pub num_constraints: usize,
    /// Number of columns in A,B,C (= `A.n_cols`).
    pub num_variables: usize,
    /// Public input length `l` (excluding the constant 1 wire).
    pub public_inputs: usize,
    /// Committed (blinded) witness length `q`.
    pub committed_witnesses: usize,
    /// Non-zero entries in matrix A.
    pub nnz_a: usize,
    /// Non-zero entries in matrix B.
    pub nnz_b: usize,
    /// Non-zero entries in matrix C.
    pub nnz_c: usize,
}

impl R1csStats {
    /// Total non-zero entries across A, B, and C.
    pub fn nnz_total(&self) -> usize {
        self.nnz_a + self.nnz_b + self.nnz_c
    }

    /// Average non-zeros per constraint row (total nnz / num_constraints).
    pub fn nnz_per_constraint(&self) -> f64 {
        if self.num_constraints == 0 {
            return 0.0;
        }
        self.nnz_total() as f64 / self.num_constraints as f64
    }
}

impl fmt::Display for R1csStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{label}] constraints={cons} variables={vars} public_io={pub_io} committed={q} \
             nnz(A,B,C)=({a},{b},{c}) total_nnz={tot} nnz/constraint={avg:.1}",
            label = self.label,
            cons = self.num_constraints,
            vars = self.num_variables,
            pub_io = self.public_inputs,
            q = self.committed_witnesses,
            a = self.nnz_a,
            b = self.nnz_b,
            c = self.nnz_c,
            tot = self.nnz_total(),
            avg = self.nnz_per_constraint(),
        )
    }
}

/// Build [`R1csStats`] from an extracted [`R1CS`].
pub fn stats_from_r1cs<F: ark_ff::PrimeField>(label: impl Into<String>, r1cs: &R1CS<F>) -> R1csStats {
    R1csStats {
        label: label.into(),
        num_constraints: r1cs.A.n_rows,
        num_variables: r1cs.A.n_cols,
        public_inputs: r1cs.l,
        committed_witnesses: r1cs.q,
        nnz_a: nnz(&r1cs.A),
        nnz_b: nnz(&r1cs.B),
        nnz_c: nnz(&r1cs.C),
    }
}

fn nnz<F: ark_ff::PrimeField>(m: &SparseMatrix<F>) -> usize {
    m.coeffs.iter().map(|row| row.len()).sum()
}

/// Pretty-print a batch of stats to stdout with a header.
pub fn print_r1cs_stats(stats: &[R1csStats]) {
    println!("=== Eva R1CS statistics ===");
    for s in stats {
        println!("  {s}");
    }
    println!("============================");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr;
    use folding_schemes::utils::vec::dense_matrix_to_sparse;

    #[test]
    fn stats_from_test_r1cs() {
        let dense = vec![
            vec![Fr::from(0u64), Fr::from(1u64), Fr::from(0u64), Fr::from(0u64), Fr::from(0u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(0u64), Fr::from(0u64), Fr::from(1u64), Fr::from(0u64), Fr::from(0u64)],
        ];
        let m = dense_matrix_to_sparse(dense);
        let r1cs = folding_schemes::ccs::r1cs::R1CS {
            l: 1,
            q: 0,
            A: m.clone(),
            B: m.clone(),
            C: m,
        };
        let stats = stats_from_r1cs("test", &r1cs);
        assert_eq!(stats.num_constraints, 2);
        assert_eq!(stats.num_variables, 6);
        assert!(stats.nnz_total() > 0);
    }
}
