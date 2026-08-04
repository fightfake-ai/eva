# wasm-prove-spike

Spike A harness: tiny Eva EditOnly brightness prove → FFPB output.

**Full documentation:** [`../docs/SPIKE_A.md`](../docs/SPIKE_A.md)

## Status

| Stage | Result |
|-------|--------|
| Native full prove | **Pass** (~3.5 min) |
| Native setup export | **Pass** → `spike-params.bin` (~1.3 GiB Groth16 PK) |
| Native prove-only (cached PK) | **Pass** (~1.7 min, toolkit verify OK) |
| WASM compile | **Pass** |
| WASM prove-only (Node) | **Fail** — OOM loading ~1.3 GiB PK into wasm linear memory |

## Phase 1 workflow

### Step 1 — Export params (native, once per toy config)

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-setup
```

Writes `spike-params.bin` (FFSP format: Groth16 proving key for the toy decider circuit).

### Step 2 — Prove-only

**Native:**

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-prove
```

**WASM (Node):**

```bash
wasm-pack build --target nodejs --features wasm-js
node pkg/run-spike.js 42
```

Requires `spike-params.bin` in `wasm-prove-spike/` (not committed — ~1.3 GiB).

### Full pipeline (baseline, includes setup)

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-native
cargo test -p wasm-prove-spike prove_only_with_cached --release --features native-bin
```

### Verify proof

```bash
cd ../../fightfake-toolkit
cargo run --release -p fightfake-cli --features eva-backend,crypto-verify -- \
  verify-proof --proof ../eva-miha/wasm-prove-spike/spike-proof.bin
```

## API (WASM)

```javascript
const params = fs.readFileSync("../spike-params.bin");
const ffpb = wasm.spike_prove_bytes_with_params(42n, params);
```

`spike_prove_bytes(seed)` still runs full setup+prove (OOM in wasm for this circuit).

## Artifacts (gitignored)

- `spike-params.bin` — cached Groth16 PK (~1.3 GiB)
- `spike-proof.bin` — FFPB output (~5.6 MB)
- `pkg/` wasm-pack output (except `pkg/run-spike.js`)
