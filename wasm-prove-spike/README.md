# wasm-prove-spike

Two prove paths share this crate:

| Path | Target | Wrap | Status |
|------|--------|------|--------|
| **Editor / in-tab** | `wasm32` + `--features wasm-js` | `Nova::verify` + Spartan(`NativePrimaryCircuit`) | **This is `/editor`** |
| Spike A (historical) | native `--features native-bin` | Groth16 `DeciderEthCircuit` | PK ~1.3 GiB; **not compiled into wasm** |

**Write-up:** [`../docs/offline-decider-and-wasm-spartan.md`](../docs/offline-decider-and-wasm-spartan.md)  
**Groth16 OOM history:** [`../docs/SPIKE_A.md`](../docs/SPIKE_A.md)

## Editor WASM (no EVM)

```bash
cd wasm-prove-spike
wasm-pack build --target web --release --features wasm-js
# copy pkg/wasm_prove_spike.js, .d.ts, _bg.wasm into
# fightfake.ai-web/public/editor-prove/  (do not overwrite worker.js)
```

JS API used by `public/editor-prove/worker.js`: `nova_start`, `nova_step`, `nova_digest`, `nova_finish`.

`nova_finish` JSON:

| `proof_system` | Meaning |
|----------------|---------|
| `nova-offline-spartan` | IVC verified (incl. CycleFold) **and** primary Spartan verified |
| `nova-ivc` | IVC verified; Spartan wrap failed (`wrap_error`) |
| `nova-wasm` | Worker caught a trap after Nova; hashes from `nova_digest` |

There are **no** `spike_prove_*` wasm exports. Groth16 lives in `src/eth_spike.rs` (`cfg(not(target_arch = "wasm32"))`).

## Test Spartan in wasm32 (Node)

Same wasm32 linear memory as a browser tab. Two stages: tiny `a*b=c` Spartan, then one 16×16 Nova fold + `NativePrimaryCircuit` wrap.

```bash
cd wasm-prove-spike
wasm-pack build --target nodejs --out-dir pkg-node --release --features wasm-js
node run-wasm-spartan.js
```

Success looks like `"wrap_ok": true` and `"proof_system": "nova-offline-spartan"`.
`RuntimeError: unreachable` is still an OOM trap.

## Spike A (native Groth16, historical)

### Step 1 — Export params (native, once per toy config)

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-setup
```

Writes `spike-params.bin` (FFSP format: Groth16 proving key for the toy ETH decider).

### Step 2 — Prove-only (native)

```bash
cargo run --release -p wasm-prove-spike --features native-bin --bin spike-a-prove
```

### Full pipeline

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

## Artifacts (gitignored)

- `spike-params.bin` — cached Groth16 PK (~1.3 GiB)
- `spike-proof.bin` — FFPB output (~5.6 MB)
- `pkg/` wasm-pack output (except committed glue if any)
