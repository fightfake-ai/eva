//! Measure Griffin and Keccak-f R1CS costs for the island-only Merkle opening debate (§7.6).
//!
//! ```bash
//! cargo run --release -p video --example hash_r1cs_cost -- \
//!   --csv docs/results/hash-r1cs-cost.csv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ark_bn254::Fr;
use ark_ff::UniformRand;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::ConstraintSystem;
use rand::thread_rng;

use video::griffin::constraints::{GriffinCircuit, PermCircuit};
use video::griffin::params::GriffinParams;
use video::keccak_r1cs::{
    keccak_f_native, measure_keccak_f_constraints, sha3_256_permutations_for_msg, SHA3_256_RATE,
};

const ENCODE_CONSTRAINTS_PER_MB: f64 = 5_589.0;

struct Row {
    primitive: &'static str,
    what: String,
    constraints: usize,
    note: String,
}

fn griffin_permute_cost(t: usize, d: usize, rounds: usize) -> Result<usize, String> {
    let params = Arc::new(GriffinParams::new(t, d, rounds));
    let cs = ConstraintSystem::<Fr>::new_ref();
    let mut rng = thread_rng();
    let state: Vec<Fr> = (0..t).map(|_| Fr::rand(&mut rng)).collect();
    let state_var = Vec::<FpVar<Fr>>::new_witness(cs.clone(), || Ok(state))
        .map_err(|e| format!("alloc: {e}"))?;
    let before = cs.num_constraints();
    let _ = GriffinCircuit::new(&params)
        .permute(&state_var)
        .map_err(|e| format!("permute: {e}"))?;
    let n = cs.num_constraints() - before;
    if !cs.is_satisfied().unwrap() {
        return Err("griffin permute unsat".into());
    }
    Ok(n)
}

fn griffin_hash_cost(t: usize, d: usize, rounds: usize, msg_fields: usize) -> Result<(usize, usize), String> {
    let params = Arc::new(GriffinParams::new(t, d, rounds));
    let rate = t - 1;
    let n_perm = msg_fields.div_ceil(rate).max(1);
    let cs = ConstraintSystem::<Fr>::new_ref();
    let mut rng = thread_rng();
    let msg: Vec<Fr> = (0..msg_fields).map(|_| Fr::rand(&mut rng)).collect();
    let msg_var = Vec::<FpVar<Fr>>::new_witness(cs.clone(), || Ok(msg))
        .map_err(|e| format!("alloc: {e}"))?;
    let before = cs.num_constraints();
    let _ = GriffinCircuit::new(&params)
        .hash(&msg_var)
        .map_err(|e| format!("hash: {e}"))?;
    let n = cs.num_constraints() - before;
    if !cs.is_satisfied().unwrap() {
        return Err("griffin hash unsat".into());
    }
    Ok((n, n_perm))
}

fn keccak_permute_cost() -> Result<usize, String> {
    let mut input = [0u64; 25];
    for i in 0..25 {
        input[i] = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xdead_beef_cafe_babe;
    }
    let mut expect = input;
    keccak_f_native(&mut expect);
    let cs = ConstraintSystem::<Fr>::new_ref();
    let (n, got) = measure_keccak_f_constraints(cs.clone(), &input)
        .map_err(|e| format!("keccak: {e}"))?;
    if got != expect {
        return Err("keccak circuit != native".into());
    }
    if !cs.is_satisfied().unwrap() {
        return Err("keccak unsat".into());
    }
    Ok(n)
}

fn main() -> ExitCode {
    let mut csv = PathBuf::from("docs/results/hash-r1cs-cost.csv");
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--csv" => {
                i += 1;
                csv = PathBuf::from(&args[i]);
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let mut rows = Vec::new();

    // --- Griffin (Eva params) ---
    let g_perm = match griffin_permute_cost(16, 5, 9) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    rows.push(Row {
        primitive: "griffin",
        what: "permute t=16 d=5 rounds=9 (Eva)".into(),
        constraints: g_perm,
        note: "one sponge permutation".into(),
    });

    // Pixel leaf as field elements: Eva packs bytes into field elems; for Merkle alternative
    // we care about hashing digests / a few field words. Measure sponge over rate (=15) fields
    // (= one permute worth of absorb) and over 2 rate blocks.
    for (label, n_fields) in [("absorb_1_rate (=15 Fr)", 15usize), ("absorb_2_rate (=30 Fr)", 30)] {
        match griffin_hash_cost(16, 5, 9, n_fields) {
            Ok((n, n_perm)) => rows.push(Row {
                primitive: "griffin",
                what: format!("hash {label}"),
                constraints: n,
                note: format!("{n_perm} permute(s); ≈ {} /perm", n / n_perm.max(1)),
            }),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Also report the unit-test params (t=24) for cross-check with existing println.
    match griffin_permute_cost(24, 5, 9) {
        Ok(n) => rows.push(Row {
            primitive: "griffin",
            what: "permute t=24 d=5 rounds=9".into(),
            constraints: n,
            note: "cross-check vs unit test".into(),
        }),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    // --- Keccak-f[1600] ---
    let k_perm = match keccak_permute_cost() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    rows.push(Row {
        primitive: "keccak_f1600",
        what: "permute (bit-oriented Boolean R1CS)".into(),
        constraints: k_perm,
        note: "unoptimized; no lookups".into(),
    });

    // Merkle-shaped SHA3-256 absorption costs (permutation count × measured perm).
    let node_msg = 3 + 32 + 32; // b"node" || left || right
    let pix_leaf_msg = 3 + 8 + 384; // b"pix" || u64 || YUV
    let syn_leaf_msg = 3 + 8 + 384 + 384 + 6; // current syntax_leaf payload
    for (label, msg_len) in [
        ("SHA3-256 Merkle node (67 B)", node_msg),
        ("SHA3-256 pixel leaf (395 B)", pix_leaf_msg),
        ("SHA3-256 syntax leaf (785 B)", syn_leaf_msg),
    ] {
        let n_perm = sha3_256_permutations_for_msg(msg_len);
        rows.push(Row {
            primitive: "sha3_256",
            what: label.into(),
            constraints: k_perm.saturating_mul(n_perm),
            note: format!(
                "{msg_len} B → {n_perm}× Keccak-f (rate {SHA3_256_RATE} B); circuit cost = n_perm × keccak_f"
            ),
        });
    }

    // Island-MB opening bill: 17 node hashes + 1 leaf (pixel), as in §7.6.
    let open_perms = 17 * sha3_256_permutations_for_msg(node_msg)
        + sha3_256_permutations_for_msg(pix_leaf_msg);
    let open_constraints = k_perm.saturating_mul(open_perms);
    rows.push(Row {
        primitive: "sha3_256",
        what: "Merkle open one island MB (17 nodes + pixel leaf)".into(),
        constraints: open_constraints,
        note: format!(
            "{open_perms} Keccak-f; {:.1}× encode/MB ({ENCODE_CONSTRAINTS_PER_MB})",
            open_constraints as f64 / ENCODE_CONSTRAINTS_PER_MB
        ),
    });

    // Same opening with Griffin: node = hash 2 digests. Digests are field elements in a
    // Griffin tree; one node = absorb 2 Fr → 1 permute. Leaf over packed pixels is more;
    // lower-bound with 1 leaf permute + 17 node permutes.
    let griffin_open = g_perm.saturating_mul(18);
    rows.push(Row {
        primitive: "griffin",
        what: "Merkle open one island MB lower bound (17 nodes + 1 leaf permute)".into(),
        constraints: griffin_open,
        note: format!(
            "18× Griffin permute; {:.3}× encode/MB ({ENCODE_CONSTRAINTS_PER_MB})",
            griffin_open as f64 / ENCODE_CONSTRAINTS_PER_MB
        ),
    });

    // Print
    println!(
        "{:<12} {:>10}  {}",
        "primitive", "constraints", "what"
    );
    println!("{}", "-".repeat(100));
    for r in &rows {
        println!(
            "{:<12} {:>10}  {}  [{}]",
            r.primitive, r.constraints, r.what, r.note
        );
    }
    println!();
    println!("Keccak-f / Griffin-permute ratio: {:.1}×", k_perm as f64 / g_perm as f64);
    println!(
        "SHA3 island-MB open / encode:      {:.1}×",
        open_constraints as f64 / ENCODE_CONSTRAINTS_PER_MB
    );
    println!(
        "Griffin island-MB open / encode:   {:.3}×",
        griffin_open as f64 / ENCODE_CONSTRAINTS_PER_MB
    );

    // CSV
    if let Some(parent) = csv.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut text = String::from(
        "primitive,what,constraints,note\n",
    );
    for r in &rows {
        text.push_str(&format!(
            "{},\"{}\",{},\"{}\"\n",
            r.primitive,
            r.what.replace('"', "'"),
            r.constraints,
            r.note.replace('"', "'")
        ));
    }
    if let Err(e) = fs::write(&csv, text) {
        eprintln!("write {}: {e}", csv.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", csv.display());
    ExitCode::SUCCESS
}
