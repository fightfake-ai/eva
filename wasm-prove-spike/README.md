# wasm-prove-spike

Spike A harness: tiny Eva EditOnly brightness prove → FFPB output.

**Full documentation:** [`../docs/SPIKE_A.md`](../docs/SPIKE_A.md)

## Status (2026-07-31)

| Stage | Result |
|-------|--------|
| Native prove + FFPB verify | **Pass** (~3.5 min, `spike-proof.bin`) |
| WASM compile | **Pass** (~2.1 MB `.wasm`) |
| WASM runtime (Node) | **Fail** — OOM during Groth16 setup (see log) |

## Quick start

### Native

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-native
cargo test -p wasm-prove-spike spike_produces --release
```

Writes `spike-proof.bin` and runs toolkit portable verify.

### WASM (Node)

```bash
wasm-pack build --target nodejs --features wasm-js
node pkg/run-spike.js 42
```

Requires nightly Rust and `wasm32-unknown-unknown`. See `../.cargo/config.toml` for getrandom + stack size.

**Note:** Current spike calls Groth16 setup in wasm and **runs out of memory**. Phase 1 should load a precomputed proving key instead.

### Verify proof (native output)

```bash
cd ../../fightfake-toolkit
cargo run --release -p fightfake-cli --features eva-backend,crypto-verify -- \
  verify-proof --proof ../eva-miha/wasm-prove-spike/spike-proof.bin
```

## Logs

- `spike-wasm-run.log` — latest Node wasm run (phase markers + error backtrace if dev build)
