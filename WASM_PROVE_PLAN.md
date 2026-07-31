# WASM prove plan (Phase 0)

Status: **investigation complete — awaiting go-ahead before Phase 1.**

Related brief: [`WASM_PROVE_AGENT_BRIEF.md`](./WASM_PROVE_AGENT_BRIEF.md).

## Scope clarification: which Eva?

| Tree | Path | Role |
|------|------|------|
| Upstream / this checkout | `/Users/miha/projects/fightfake-projects/eva` (`winderica/eva`, branch `video`) | CUDA-oriented; no `EditOnlyCircuit` / `RedactRect` |
| Fightfake fork | `/Users/miha/projects/fightfake-projects/eva-miha` (`fightfake-ai/eva`) | What toolkit `eva-backend` pins; CPU default; edit-only + redact |

**Recommendation:** implement WASM prove against the **fightfake fork**. This checkout (upstream) is the wrong code surface for toolkit-compatible demos. Either move this plan into the fork, or treat the fork as the implementation repo and keep this file as the decision record.

---

## How native prove works today

```
pixels (macroblocks)
    → video::EditOnlyCircuit<Fr, Brightness|RedactRect|…>   # F-circuit (toolkit path)
    → folding-schemes::Nova::{preprocess, init, prove_step×N}
    → video::decider::Decider::prove                         # Groth16 “onchain” decider
    → toolkit FFPB proof.bin  (fightfake-core::proof_bundle)
```

Toolkit glue: `fightfake-cli` `--features eva-backend` (`workflow.rs`).  
WASM verify today: `fightfake-core` `crypto-verify` (arkworks-only reimplementation of decider verify — **no** Eva dep).

---

## Blocker inventory (wasm32-unknown-unknown)

### Empirically checked (fightfake fork, 2026-07-31)

Probe crate depending on `video` / `folding-schemes` with:

- `--target wasm32-unknown-unknown`
- `default-features = false`, `features = ["cpu"]` (implies `parallel` + CUDA CPU stubs)
- `getrandom` `js` feature + `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'`

**Result: `cargo check` succeeds** for both `folding-schemes` and `video` on nightly (`1.97.0-nightly`).

That updates prior toolkit wording (“cannot target wasm32”). Compile is possible with the right features; the remaining risks are **runtime**, **nightly**, and **resource limits**.

### With `--no-default-features` (no `cpu`)

Build **fails**:

- Missing `CudaStream` / `DeviceVec` (CPU stubs live behind `cpu`)
- Missing `PreparedMatrix` / broken serial espresso sum-check paths
- Confirms toolkit note: non-parallel fallback does not compile; `cpu = ["parallel"]` is mandatory

### Remaining blockers / risks

| Item | Severity | Notes |
|------|----------|--------|
| **`#![feature(test)]` in `video/src/lib.rs`** | High for stable | Requires **nightly**. Must gate/remove for `wasm-pack` on stable. |
| **`getrandom` on wasm** | Low (solved pattern) | Same as toolkit WASM verify: `js` + `getrandom_backend="wasm_js"`. |
| **`rayon` at runtime** | Medium–High | Compiles under `cpu`/`parallel`. On `wasm32-unknown-unknown` without atomics/SharedArrayBuffer, rayon typically serializes or needs `wasm-bindgen-rayon`. **Not yet runtime-tested.** |
| **`ark-ff` `asm` feature** | Low (compile OK) | Enabled in Cargo.toml; wasm check still succeeded (asm crate compiled). Keep watching for linker/runtime issues. |
| **CUDA / icicle** | None if `cpu` only | Optional on fork; do not enable `cuda` for WASM. |
| **Circuit size / RAM / time** | High (product) | Full clips impossible in-tab. Toy must stay tiny (see below). Native edit-encode path ≈ **1.43M constraints/step** at 256 MBs/step; edit-only is smaller but Nova+Groth16 setup still heavy. |
| **Per-proof Groth16 setup** | High | `Decider::prove` runs `generate_random_parameters_with_reduction` each prove — large cost in-browser. |
| **File I/O / ffmpeg** | None for prove math | Supply macroblocks from JS; leave ingest outside WASM. |

### Upstream (this repo) vs fork

Upstream still hard-wires CUDA-oriented deps more aggressively and lacks edit-only. Do **not** start the WASM port here.

---

## Essential vs optional crates (toy prove)

| Keep | Drop / avoid |
|------|----------------|
| `folding-schemes` (`cpu`, no `cuda`) | `cuda`, icicle |
| `video` edit-only + Griffin + Brightness or RedactRect | `EditEncodeCircuit` / encode constraint path |
| Groth16 decider **if** FFPB compatibility is required | Circom frontend, comparison crate, file examples |
| In-memory macroblock tiling (or JS-side tiling) | ffmpeg inside WASM |

---

## Options

### A — Port / wrap Eva folding prove for wasm32 (single-threaded / browser)

**Feasibility (updated):** compile path looks open on the fork with `cpu` + getrandom. Main work:

1. Gate/remove `#![feature(test)]` (stable builds).
2. Document wasm `getrandom` flags (mirror toolkit).
3. Confirm rayon runtime (serial OK vs need `wasm-bindgen-rayon` + atomics).
4. New feature `wasm-prove` + thin API: toy EditOnly 1-step (+ optional decider).
5. Emit FFPB (or document delta) for toolkit `crypto-verify`.

**Pros:** Same circuits / FFPB / org.zkedit story as native `eva-backend`.  
**Cons:** Still a large stack; Groth16 setup + Nova may blow browser time/RAM even for toys; rayon runtime unknown until tried.

**Effort estimate:**  
- If rayon runs serialized and a 1-step edit-only toy proves: **~2–4 days** to a wasmtime/browser demo.  
- If rayon/atomics or Groth16 setup force a deeper serial rewrite: **1–3+ weeks** → fall back to B.

**Stop rule (from brief):** if after a short runtime spike it is clear that Eva-in-WASM needs a multi-week rewrite, switch to B.

### B — Minimal arkworks circuit (toy brightness/redact), new proof format

Standalone Groth16 (or similar) over a tiny bitmap / few macroblocks; semantics inspired by Eva gadgets; **not** bit-compatible FFPB unless carefully engineered.

**Pros:** Smallest vertical slice; reuses toolkit’s known-good wasm arkworks path.  
**Cons:** Different `proof_system`; migration story back to Eva needed; duplicate gadget logic.

**Effort estimate:** **~1–3 days** to prove+verify a toy in WASM.

### C — Other

e.g. prove natively / server-side and only verify in WASM (already done). Does **not** meet the brief’s “produce a real proof under wasm32” goal.

---

## Recommendation

**Start with a short Option A spike on the fightfake fork** (compile already green):

1. Tiny native-equivalent harness: `EditOnlyCircuit` + Brightness, **1 Nova step**, ≤16 macroblocks (e.g. 64×64 still = 16 MBs), measure wall time + RSS on laptop CPU.
2. Same harness under `wasm32` via `wasm-bindgen` or install `wasmtime` and run — confirm rayon does not panic.
3. **Go / no-go:**
   - If toy proves in ~≤60s in wasmtime/browser with acceptable RAM → continue A through FFPB + toolkit verify.
   - If blocked on threads/setup/memory → **pivot to B** immediately and keep A as a longer native-compat track.

Do **not** begin a multi-week folding-schemes refactor before that spike.

Default product stance if A fails the spike: **B for in-tab demo proofs**; native Eva remains the production prover; WASM `crypto-verify` remains the production verifier for FFPB.

---

## Default toy instance (Phase 1 target)

| Parameter | Value |
|-----------|--------|
| Media | Still image, **64×64** YUV420 (16 macroblocks) or **16×16** (1 MB) stretch |
| Gadget | **Brightness** (`BrightnessCfg`, fixed scale) |
| IVC | **1** `prove_step` (`blocks_per_step = N`) |
| Circuit | `EditOnlyCircuit` (no encode) |
| Output | Prefer FFPB verifiable by toolkit `crypto-verify`; if setup cost too high, Nova-only + documented verify for demo, then add decider |
| Time budget | ≤60s browser/wasmtime (stretch ≤10s) |

Public inputs / assertions: keep `h1`, `h2`, `gadget_id` aligned with `org.zkedit.edit_proof` when emitting FFPB.

---

## Phase 1 outline (only after approval)

1. Work in **eva-miha** / `fightfake-ai/eva` (not upstream CUDA tree).
2. Feature-gate `wasm-prove`; remove or `cfg`-gate `feature(test)`.
3. Spike runtime (native size → wasm32).
4. If green: `eva-wasm-prove` (or toolkit `crypto-prove`) with `wasm-bindgen` entrypoint; document build flags.
5. Verify with existing `verifyGroth16Proof` / `verify-proof` when FFPB-shaped.

## Phase 2

Glue into toolkit WASM package behind `crypto-prove` / `wasm-prove`, without pulling Eva into default verify builds.

---

## Definition of done (unchanged)

A developer can follow docs, build a WASM package, call prove on the toy input (browser or wasmtime), and verify with the existing or newly documented verifier path.

---

## Ask for approval

**Spike A completed** — see [`docs/SPIKE_A.md`](docs/SPIKE_A.md) for full results.

Summary:

- **Compile:** Eva prove stack builds for `wasm32` with `cpu` feature (no CUDA) + getrandom JS + nightly.
- **Native prove:** QUICK toy (4 MBs/step × 2 steps) completes in ~3.5 min; FFPB verifies via toolkit.
- **WASM runtime:** Nova preprocess OK; **Groth16 setup OOM** in wasm linear memory (~4–6 min before trap). Dev build backtrace: `alloc::rust_oom` in `ark_relations` during trusted setup. Browser not tested.
- **Phase 1 prerequisite:** precompute Groth16 PK offline; do not run setup in wasm.

Please confirm whether to proceed with **Phase 1** (cached Groth16 params, wasm prove-only retry, `fightfake-wasm` glue) per SPIKE_A recommendation.
