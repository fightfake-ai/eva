//! Native Spike A runner — prints JSON report and writes `spike-proof.bin`.
//!
//! ```bash
//! cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-native
//! ```

use std::fs;
use std::path::PathBuf;

use wasm_prove_spike::{run_spike, SpikeConfig};

fn main() {
    let config = SpikeConfig::default();
    println!("=== Spike A (native) ===");
    println!("config: {}", serde_json::to_string_pretty(&config).unwrap());

    let result = match run_spike(config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("SPIKE FAILED: {e}");
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
        eprintln!("SPIKE FAILED: FFPB header missing");
        std::process::exit(1);
    }

    // Portable verify (production path — same as fightfake verify-proof / WASM extension).
    #[cfg(feature = "native-bin")]
    verify_with_toolkit(&result.proof_bytes);

    println!("\nOK — proof written to {}", out_path.display());
    println!("Verify with toolkit:");
    println!(
        "  cd ../../fightfake-toolkit && cargo run -p fightfake-cli --features eva-backend,crypto-verify -- verify-proof --proof ../eva-miha/wasm-prove-spike/spike-proof.bin"
    );
}

#[cfg(feature = "native-bin")]
fn verify_with_toolkit(proof_bytes: &[u8]) {
    use fightfake_core::proof_bundle::{verify_proof_bundle, ProofBundle};

    let bundle = ProofBundle::from_bytes(proof_bytes).expect("parse FFPB");
    let ok = verify_proof_bundle(&bundle).expect("toolkit verify");
    if !ok {
        eprintln!("SPIKE FAILED: toolkit portable verify returned false");
        std::process::exit(1);
    }
    println!("toolkit portable verify: OK");
}
