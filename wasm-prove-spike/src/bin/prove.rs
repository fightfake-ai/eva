//! Prove-only runner using cached `spike-params.bin`.
//!
//! ```bash
//! cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-prove
//! ```

use std::fs;
use std::path::PathBuf;

use wasm_prove_spike::{run_prove, SpikeConfig, SpikeParams};

fn main() {
    let config = SpikeConfig::default();
    let params_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spike-params.bin");
    let params_bytes = fs::read(&params_path).unwrap_or_else(|e| {
        eprintln!("Missing {params_path:?}: run spike-a-setup first ({e})");
        std::process::exit(1);
    });
    let cached = SpikeParams::from_bytes(&params_bytes).unwrap_or_else(|e| {
        eprintln!("Invalid spike-params.bin: {e}");
        std::process::exit(1);
    });

    println!("=== Spike A prove-only (native) ===");
    println!("config: {}", serde_json::to_string_pretty(&config).unwrap());
    println!("params: {} ({} bytes)", params_path.display(), params_bytes.len());

    let result = match run_prove(config, &cached) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("PROVE FAILED: {e}");
            std::process::exit(1);
        }
    };

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spike-proof.bin");
    fs::write(&out_path, &result.proof_bytes).expect("write spike-proof.bin");

    let mut report = serde_json::to_value(&result).expect("json");
    if let Some(obj) = report.as_object_mut() {
        obj.remove("proof_bytes");
        obj.insert(
            "proof_bin_path".into(),
            serde_json::Value::String(out_path.display().to_string()),
        );
    }

    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    if !result.ffpb_header_ok {
        eprintln!("PROVE FAILED: FFPB header missing");
        std::process::exit(1);
    }

    verify_with_toolkit(&result.proof_bytes);
    println!("\nOK — proof written to {}", out_path.display());
}

fn verify_with_toolkit(proof_bytes: &[u8]) {
    use fightfake_core::proof_bundle::{verify_proof_bundle, ProofBundle};

    let bundle = ProofBundle::from_bytes(proof_bytes).expect("parse FFPB");
    let ok = verify_proof_bundle(&bundle).expect("toolkit verify");
    if !ok {
        eprintln!("PROVE FAILED: toolkit portable verify returned false");
        std::process::exit(1);
    }
    println!("toolkit portable verify: OK");
}
