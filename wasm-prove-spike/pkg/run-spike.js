// Run Spike A prove under Node (wasm-pack nodejs target).
// Usage: node run-spike.js [seed]
const fs = require("fs");
const path = require("path");

const seed = BigInt(process.argv[2] ?? "42");
const wasm = require("./wasm_prove_spike.js");

async function main() {
  console.error("WASM Spike A starting (expect several minutes)…");
  const t0 = Date.now();
  const bytes = wasm.spike_prove_bytes(seed);
  const ms = Date.now() - t0;

  const proofPath = path.join(__dirname, "spike-proof-wasm.bin");
  fs.writeFileSync(proofPath, Buffer.from(bytes));

  console.log(
    JSON.stringify(
      {
        wall_clock_ms_node: ms,
        proof_bytes_len: bytes.length,
        proof_bin_path: proofPath,
        ffpb_header_ok: bytes[0] === 0x46 && bytes[1] === 0x46 && bytes[2] === 0x50 && bytes[3] === 0x42,
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
