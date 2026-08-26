//! Publishable Spartan VKs for the editor wrap (`NativePrimaryCircuit`).
//!
//! Shape depends on gadget + blocks-per-step, not on the photo. Editor uses
//! `bps=4` when the macroblock count is divisible by 4.
//!
//! ```bash
//! cargo run --release -p wasm-prove-spike --bin export-offline-vk -- \
//!   /path/to/fightfake.ai-web/public/verify/vk
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use wasm_prove_spike::{nova_export_vk, nova_start};

fn dummy_rgba(w: usize, h: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..(w * h) {
        let o = i * 4;
        rgba[o] = (i % 251) as u8;
        rgba[o + 1] = ((i * 3) % 251) as u8;
        rgba[o + 2] = 90;
        rgba[o + 3] = 255;
    }
    rgba
}

fn export_one(gadget: &str, a: u32, b: u32, c: u32, d: u32, out_dir: &PathBuf) {
    let w = 64u32;
    let h = 16u32;
    let rgba = dummy_rgba(w as usize, h as usize);
    let t0 = Instant::now();
    let info = nova_start(&rgba, w, h, gadget, a, b, c, d).expect("nova_start");
    eprintln!("[{gadget}] nova_start {} ms: {info}", t0.elapsed().as_millis());

    let t1 = Instant::now();
    let (vk_bytes, meta) = nova_export_vk().expect("nova_export_vk");
    eprintln!(
        "[{gadget}] setup {} ms, vk {} bytes, cons {} pad {} sha256 {}",
        t1.elapsed().as_millis(),
        meta.vk_bytes_len,
        meta.num_constraints,
        meta.num_constraints_padded,
        meta.vk_sha256
    );

    fs::create_dir_all(out_dir).expect("mkdir vk dir");
    let bin_path = out_dir.join(format!("{}.bin", meta.circuit_id));
    fs::write(&bin_path, &vk_bytes).expect("write vk bin");
    let meta_path = out_dir.join(format!("{}.json", meta.circuit_id));
    fs::write(
        &meta_path,
        serde_json::to_string_pretty(&meta).expect("meta json") + "\n",
    )
    .expect("write vk meta");
    eprintln!("wrote {}", bin_path.display());
}

fn main() {
    let out_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "offline-vk".into()),
    );
    eprintln!("export-offline-vk → {}", out_dir.display());
    export_one("redact", 0, 0, 16, 16, &out_dir);
    export_one("brightness", 1024, 0, 0, 0, &out_dir);
    eprintln!("done");
}
