// Run Spike A prove-only under Node (requires spike-params.bin from spike-a-setup).
// Usage: node run-spike.js [seed]
const fs = require("fs");
const path = require("path");

const seed = BigInt(process.argv[2] ?? "42");
const wasm = require("./wasm_prove_spike.js");

const paramsPath = path.join(__dirname, "..", "spike-params.bin");
if (!fs.existsSync(paramsPath)) {
  console.error(`Missing ${paramsPath}`);
  console.error("Run: cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-setup");
  process.exit(1);
}

const paramsBytes = fs.readFileSync(paramsPath);

async function main() {
  console.error("WASM Spike A prove-only (cached params)…");
  const t0 = Date.now();
  const bytes = wasm.spike_prove_bytes_with_params(seed, paramsBytes);
  const ms = Date.now() - t0;

  const proofPath = path.join(__dirname, "spike-proof-wasm.bin");
  fs.writeFileSync(proofPath, Buffer.from(bytes));

  console.log(
    JSON.stringify(
      {
        wall_clock_ms_node: ms,
        proof_bytes_len: bytes.length,
        proof_bin_path: proofPath,
        params_bytes_len: paramsBytes.length,
        ffpb_header_ok:
          bytes[0] === 0x46 && bytes[1] === 0x46 && bytes[2] === 0x50 && bytes[3] === 0x42,
      },
      null,
      2,
    ),
  );
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
