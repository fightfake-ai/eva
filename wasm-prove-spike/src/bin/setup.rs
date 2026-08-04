//! Export offline Groth16 params for prove-only wasm runs.
//!
//! ```bash
//! cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-setup
//! ```

use std::fs;
use std::path::PathBuf;

use wasm_prove_spike::{run_setup, SpikeConfig};

fn main() {
    let config = SpikeConfig::default();
    println!("=== Spike A setup (native) ===");
    println!("config: {}", serde_json::to_string_pretty(&config).unwrap());

    let result = match run_setup(config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("SETUP FAILED: {e}");
            std::process::exit(1);
        }
    };

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spike-params.bin");
    let bytes = result.params.to_bytes().expect("serialize params");
    fs::write(&out_path, &bytes).expect("write spike-params.bin");

    let mut report = serde_json::to_value(&result).expect("json");
    if let Some(obj) = report.as_object_mut() {
        obj.insert(
            "params_bin_path".into(),
            serde_json::Value::String(out_path.display().to_string()),
        );
    }

    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    println!("\nOK — params written to {} ({} bytes)", out_path.display(), bytes.len());
    println!("Prove-only (native):");
    println!("  cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-prove");
    println!("WASM (Node):");
    println!("  wasm-pack build --target nodejs --features wasm-js && node pkg/run-spike.js 42");
}
