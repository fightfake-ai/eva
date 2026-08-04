# WASM prove plan

Status: **Spike A complete** — see [`docs/SPIKE_A.md`](docs/SPIKE_A.md). Awaiting go-ahead for Phase 1.

Related brief: [`WASM_PROVE_AGENT_BRIEF.md`](./WASM_PROVE_AGENT_BRIEF.md).

## How prove works today

```
pixels (macroblocks)
    → video::EditOnlyCircuit<Fr, Brightness|RedactRect|…>
    → folding-schemes::Nova::{preprocess, init, prove_step×N}
    → video::decider::Decider::prove          # Groth16 decider
    → FFPB proof.bin  (fightfake-core::proof_bundle)
```

Toolkit glue: `fightfake-cli` with `--features eva-backend` (`workflow.rs`).

WASM verify today: `fightfake-core` `crypto-verify` — standalone arkworks verifier (see explanation in SPIKE_A FAQ / below).

---

## Spike A results (2026-07-31)

| Stage | Result |
|-------|--------|
| Compile for `wasm32-unknown-unknown` | **Pass** (with `cpu` feature + getrandom JS + nightly) |
| Native prove (QUICK toy) | **Pass** (~3.5 min, FFPB verifies) |
| WASM runtime (Node) | **Fail** — OOM during Groth16 trusted setup |

Details: [`docs/SPIKE_A.md`](docs/SPIKE_A.md).

**Phase 1 prerequisite:** precompute Groth16 proving key offline; do not run setup inside wasm.

---

## Remaining risks for browser prove

| Item | Notes |
|------|--------|
| `#![feature(test)]` in `video` | Requires nightly until gated/removed |
| `getrandom` on wasm | Solved: `js` feature + `getrandom_backend="wasm_js"` |
| `rayon` at runtime | Compiles; browser threading still needs validation |
| Circuit size / RAM / time | Full clips impossible in-tab; keep toy config |
| Per-proof Groth16 setup | Too heavy for in-tab; must cache PK offline |
| ffmpeg / file I/O | Out of scope for prove math; feed macroblocks from JS |

---

## Options considered

### A — Port Eva folding prove to wasm32

Run the same Nova + Groth16 decider stack in the browser, emit FFPB compatible with toolkit verify.

**Pros:** Same circuits and proof format as native `eva-backend`.  
**Cons:** Large memory footprint; Groth16 setup cannot run in wasm for this circuit size.

### B — Minimal standalone arkworks circuit

Tiny brightness/redact toy circuit with a new proof format.

**Pros:** Smaller, faster to demo in WASM.  
**Cons:** Not bit-compatible FFPB; duplicate gadget logic.

### C — Prove server-side, verify in WASM

Already works today. Does not meet the goal of producing proofs under wasm32.

---

## Phase 1 outline

1. ~~Precompute Groth16 PK/VK offline~~ — **done** (`spike-a-setup` → `spike-params.bin`, ~1.3 GiB).
2. ~~Re-run wasm spike with cached PK~~ — **native pass**, **wasm fail** (PK too large for linear memory).
3. Browser smoke test — blocked until PK size reduced.
4. Expose via `fightfake-wasm` behind `crypto-prove` / `wasm-prove`.
5. Verify with existing `verifyGroth16Proof` / `verify-proof`.

## Phase 2

Integrate into the toolkit WASM package without pulling Eva into default verify builds.

---

## Definition of done

A developer can build a WASM package, call prove on the toy input (browser or Node), and verify with the existing verifier path.

---

## Next step

Confirm whether to proceed with **Phase 1** (cached Groth16 params, wasm prove-only retry, `fightfake-wasm` glue).
