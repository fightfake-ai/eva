#!/usr/bin/env node
// wasm32 Spartan smoke: tiny circuit, then 16×16 Nova + NativePrimaryCircuit wrap.
//
//   wasm-pack build --target nodejs --out-dir pkg-node --release --features wasm-js
//   node run-wasm-spartan.js
//
// Same linear-memory limits as a browser tab. Does not load Groth16 / DeciderEthCircuit.

const path = require("path");
const wasm = require(path.join(__dirname, "pkg-node", "wasm_prove_spike.js"));

function log(phase, extra) {
  const t = ((Date.now() - t0) / 1000).toFixed(1);
  console.error(`[${t}s] ${phase}${extra ? " " + extra : ""}`);
}

const t0 = Date.now();

function runTiny() {
  log("A  spartan_tiny_smoke (a*b=c, no Nova)");
  const raw = wasm.spartan_tiny_smoke();
  const result = JSON.parse(raw);
  log("A  done", JSON.stringify(result));
  if (!result.verified) {
    throw new Error("tiny Spartan verified=false");
  }
  return result;
}

function runWrap16() {
  const w = 16;
  const h = 16;
  const rgba = new Uint8Array(w * h * 4);
  for (let i = 0; i < rgba.length; i++) rgba[i] = (i * 17) & 255;

  log("B  nova_start 16×16 brightness");
  const info = JSON.parse(wasm.nova_start(rgba, w, h, "brightness", 416, 0, 0, 0));
  log("B  session", JSON.stringify(info));

  for (let i = 0; i < info.steps; i++) {
    log("B  nova_step", `${i + 1}/${info.steps}`);
    wasm.nova_step();
  }

  log("B  nova_digest");
  const digest = JSON.parse(wasm.nova_digest());
  log("B  digest", JSON.stringify({ h1: digest.h1.slice(0, 16), h2: digest.h2.slice(0, 16) }));

  log("B  nova_finish (Nova::verify + Spartan NativePrimaryCircuit)");
  let done;
  try {
    done = JSON.parse(wasm.nova_finish());
  } catch (err) {
    const msg = err && err.message ? err.message : String(err);
    log("B  trapped", msg);
    return { trapped: true, wrap_error: msg, digest };
  }
  log("B  done", JSON.stringify(done));
  return done;
}

function main() {
  const tiny = runTiny();
  const wrap = runWrap16();
  const report = {
    wall_clock_s: (Date.now() - t0) / 1000,
    tiny,
    wrap,
    spartan_tiny_ok: Boolean(tiny.verified),
    wrap_ok: wrap && wrap.proof_system === "nova-offline-spartan" && wrap.verified === true,
  };
  console.log(JSON.stringify(report, null, 2));
  if (!report.spartan_tiny_ok || !report.wrap_ok) {
    process.exitCode = 1;
  }
}

try {
  main();
} catch (err) {
  console.error(err);
  process.exit(1);
}
