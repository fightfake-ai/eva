# Spike A — WASM prove feasibility (results)

**Repo:** `eva-miha` (`fightfake-ai/eva` fork)  
**Crate:** `wasm-prove-spike/`  
**Date:** 2026-07-31  
**Status:** **Complete** — compile + native prove green; WASM runtime blocked on memory during Groth16 setup.

## Goal

Validate **Option A**: run a tiny real Eva prove (`EditOnlyCircuit` + Brightness → Nova IVC → Groth16 decider) under `wasm32-unknown-unknown`, producing an **FFPB** blob verifiable by fightfake-toolkit `crypto-verify`.

## Toy instance

| Parameter | Value |
|-----------|--------|
| Input | Synthetic random macroblocks (no ffmpeg / no `DATA_PATH`) |
| Gadget | `BrightnessCfg(416)` |
| `blocks_per_step` | 4 (Eva `QUICK=1` scale) |
| `num_steps` | 2 |
| Circuit | `EditOnlyCircuit` (no H.264 encode) |
| Output | FFPB v1 `proof.bin` (~5.6 MB for this run) |

**Note:** A single IVC step was attempted first; native `Decider::verify` in Eva panics when normalizing commitment points at infinity. Two steps with distinct macroblocks per step works for prove + portable verify.

## Build configuration

### Native

```bash
cd /path/to/eva-miha
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-native
cargo test -p wasm-prove-spike spike_produces --release
```

### WASM (Node.js)

```bash
cd wasm-prove-spike
wasm-pack build --target nodejs --features wasm-js
node pkg/run-spike.js 42
```

**Requirements:**

- Rust **nightly** (Eva `video` still uses `#![feature(test)]` for benches; gated off on `wasm32`)
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- Features: `video` / `folding-schemes` with `default-features = false`, `features = ["cpu"]` (CUDA off; CPU stubs on; **`trace` off** — see below)
- `getrandom` JS backend: `getrandom` crate `js` feature + `.cargo/config.toml`:

```toml
[target.wasm32-unknown-unknown]
rustflags = [
  '--cfg', 'getrandom_backend="wasm_js"',
  '-C', 'link-arg=-zstack-size=33554432',  # 32 MiB stack
]
```

### WASM-specific code changes (this spike)

| Change | Why |
|--------|-----|
| `video/src/lib.rs` — `#![feature(test)]` gated with `#[cfg(not(target_arch = "wasm32"))]` | Stable-ish wasm build |
| `folding-schemes` — `trace` feature (default on native, **off** for wasm spike deps) | `ark-std/print-trace` uses `Instant`, which panics on wasm |
| `wasm-prove-spike/src/lib.rs` — `Instant` timing gated off on `wasm32` | Same `time not implemented` panic |
| `folding-schemes` / `video` — `asm` feature optional (default on native, off via `cpu`-only wasm deps) | Safer wasm link |
| Phase logging via `web_sys::console` in `run_spike()` | Pinpoint runtime failure stage |

## Results (Apple Silicon laptop, 2026-07-31)

### Compile

| Target | Command | Result |
|--------|---------|--------|
| `wasm32` + `cpu` only | `cargo check -p video --no-default-features --features cpu --target wasm32` | **OK** (with getrandom cfg) |
| `wasm32` without `cpu` | same, no `cpu` | **Fails** (missing CUDA CPU stubs) |
| WASM artifact | `wasm-pack build --target nodejs --features wasm-js` | **Success** |
| WASM artifact size | `pkg/wasm_prove_spike_bg.wasm` | **~2.1 MB** (release, wasm-opt) |

Prior toolkit comment (“cannot target wasm32”) referred to builds **without** the `cpu` feature. With `cpu`, the full Eva prove stack **compiles** for WASM.

### Native prove (seed=42)

| Phase | Time |
|-------|------|
| Nova preprocess | ~2.5 s |
| Groth16 setup (`generate_random_parameters_with_reduction`) | ~115 s |
| Nova IVC prove (2 steps × 4 MBs) | ~0.4 s |
| Groth16 decider prove | ~93 s |
| **Total** | **~211 s (~3.5 min)** |

Output: `wasm-prove-spike/spike-proof.bin` (5,617,277 bytes), FFPB header `FFPB\x01`.

### Verification (native proof)

```bash
cd fightfake-toolkit
cargo run --release -p fightfake-cli --features eva-backend,crypto-verify -- \
  verify-proof --proof ../eva-miha/wasm-prove-spike/spike-proof.bin
```

**Result:** `ok — Groth16 pairing check passed`

Unit test `spike_produces_ffpb_verifiable_by_toolkit` also passes via `fightfake_core::proof_bundle::verify_proof_bundle`.

### WASM runtime (Node.js)

Log: `wasm-prove-spike/spike-wasm-run.log`

| Observation | Detail |
|-------------|--------|
| Nova preprocess | **Completes** (~few seconds) |
| Groth16 setup | **Fails** after ~4–6 min wall time |
| Nova prove / Groth16 prove | **Never reached** |
| Release build error | `RuntimeError: unreachable` (OOM compiled to trap) |
| Dev build error | **`alloc::rust_oom`** while growing `ark_relations::r1cs::LinearCombination` during Groth16 trusted setup |
| `NODE_OPTIONS=--max-old-space-size=16384` | No effect (Node heap ≠ wasm linear memory) |
| 32 MiB wasm stack (`-zstack-size=33554432`) | No effect (not a stack overflow) |

**Root cause:** **WASM linear memory exhaustion** during per-proof Groth16 setup for the decider circuit. The decider R1CS is large; `generate_random_parameters_with_reduction` allocates constraint-system matrices that exceed practical in-process wasm memory during the spike run.

Phase markers emitted before failure:

```
spike: start
spike: nova preprocess
spike: groth16 setup
→ OOM / unreachable
```

## Findings

### Go for Option A — with strong caveats

1. **Compile path is open** on the fightfake fork with `cpu` + getrandom JS + nightly.
2. **Native end-to-end works** at QUICK scale: Nova + Groth16 → FFPB verifies via toolkit portable path.
3. **Dominant cost is Groth16 per-proof setup** (~115 s native), not Nova IVC (~0.4 s). Any browser demo **must** ship precomputed proving key / SRS — generating it in-tab is infeasible for this circuit size.
4. **WASM runtime fails at Groth16 setup (OOM)** even in Node with generous host heap. Fixing this requires **offline setup + loading cached params** (and likely still careful memory budgeting for prove).
5. **Proof bundle is ~5.6 MB** — too large for inline C2PA assertion; same as native `eva-backend` today.
6. **Native `Decider::verify` panics** on some commitment infinity points; **toolkit `verify_proof_bundle` handles them** (`xy().unwrap_or(zero)`). Production verify path is already the portable one.
7. **`ark-std/print-trace`** must stay disabled on wasm (uses `Instant`).

### Not ready yet

| Item | Blocker |
|------|---------|
| In-tab prove (browser or Node wasm) | OOM at Groth16 setup; need cached PK + memory plan |
| Sub-60 s prove | Groth16 setup dominates even on native CPU |
| Stable Rust | `#![feature(test)]` still on native benches |
| Browser tab | Not tested; Node wasm failed first |
| 1-step minimal instance | 2-step QUICK config used (1-step hits infinity in native self-verify) |

## Code map

| Path | Purpose |
|------|---------|
| `wasm-prove-spike/src/lib.rs` | `run_spike()`, FFPB serialization, `#[wasm_bindgen]` exports, phase logging |
| `wasm-prove-spike/src/bin/native.rs` | Native runner + toolkit verify |
| `wasm-prove-spike/pkg/` | `wasm-pack` output (`spike_prove_json`, `spike_prove_bytes`) |
| `wasm-prove-spike/pkg/run-spike.js` | Node driver |
| `wasm-prove-spike/spike-proof.bin` | Native FFPB output (gitignored if large) |
| `wasm-prove-spike/spike-wasm-run.log` | Latest WASM runtime log |
| `video/src/lib.rs` | `#![feature(test)]` gated off on `wasm32` |
| `folding-schemes/Cargo.toml` | `trace` feature for `print-trace` (native default) |
| `.cargo/config.toml` | getrandom + wasm stack size |

## API (WASM)

```javascript
const wasm = require("./wasm_prove_spike.js");
const report = JSON.parse(wasm.spike_prove_json(42n));
const ffpbBytes = wasm.spike_prove_bytes(42n);
```

## Recommendation after Spike A

**Phase 1 Step 1+2 implemented** (see `wasm-prove-spike/`):

1. **`spike-a-setup`** — native export of Groth16 PK → `spike-params.bin` (FFSP format, ~1.3 GiB).
2. **`run_prove` / `spike-a-prove` / `spike_prove_bytes_with_params`** — prove-only using cached PK.

**Results:**

| Step | Result |
|------|--------|
| Native setup export | **Pass** (~2 min) |
| Native prove-only + toolkit verify | **Pass** (~1.7 min) |
| WASM prove-only (Node) | **Fail** — OOM deserializing ~1.3 GiB PK into wasm memory (~40 min → trap) |

Groth16 **setup** OOM in wasm is avoided by caching, but the **proving key itself** is too large for current wasm32 linear memory. Next options: smaller decider circuit, server-side prove, or streaming/chunked PK loading (research).

**Continue Option A** for product integration on native prove path; browser prove needs PK size reduction before Step 3 (browser smoke test) is meaningful.

## Related docs

- [`WASM_PROVE_AGENT_BRIEF.md`](../WASM_PROVE_AGENT_BRIEF.md) — original brief
- [`WASM_PROVE_PLAN.md`](../WASM_PROVE_PLAN.md) — Phase 0 plan (updated)
- [`wasm-prove-spike/README.md`](../wasm-prove-spike/README.md) — quick commands
