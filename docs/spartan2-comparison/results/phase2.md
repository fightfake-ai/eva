# Phase 2 results — LogUp + NeutronNova (2026-08-06)

## Environment

- Machine: developer Mac (darwin)
- Branch: `spartan2-comparison` (synced with `video`)
- Profile: `--release`, features: `cpu`
- Circuit: bellpepper LogUp only (table `0..255`), **not** full Eva encode/edit
- Challenge: fixed witness `c = 1_000_003` (FS binding deferred — see LOOKUPS.md)

## Smoke runs

| NUM_QUERIES | Meaning | Constraints | setup | prep | prove (batch 2) | verify |
|-------------|---------|-------------|-------|------|-----------------|--------|
| 16 | unit / CI | 274 (core≈273) | 128 ms | 0.1 ms | 22 ms | 27 ms |
| **384** | **1 MB pixels** (Y+U+V) | **642** (core≈641) | 66 ms | 0.1 ms | **31 ms** | 17 ms |
| **2320** | **1 MB NoOp committed-query scale** | **2578** (core≈2577) | 72 ms | 0.1 ms | **59 ms** | 20 ms |

Commands:

```bash
NUM_QUERIES=16 NUM_STEPS=2 cargo run --release -p comparison --example phase2_lookup_smoke
NUM_QUERIES=384 NUM_STEPS=2 cargo run --release -p comparison --example phase2_lookup_smoke
NUM_QUERIES=2320 NUM_STEPS=2 cargo run --release -p comparison --example phase2_lookup_smoke
```
Unit tests:

```bash
cargo test -p comparison bellpepper --release
```

## Fidelity checks

| Check | Result |
|-------|--------|
| Constraint formula `Q + T + 1` (+1 public IO) | Pass (274 ≈ 16+256+1+1; 642 ≈ 384+256+1+1) |
| Histogram multiplicities | Pass (unit test) |
| NeutronNova verify | Pass for Q=16, Q=384, and Q=2320 |
| Matches Eva 1-MB step constraint count (6,117) | **Partial** — LogUp core ≈2577 of ~6117; encode/hash still missing |
| FS challenge `c = Poseidon(cmQ)` | **Deferred** |

## Interpretation

- Option A (bellpepper LogUp) works at full **1-MB committed-query scale** (Q=2320)
  under NeutronNova (~30 ms/step amortized for lookup-only).
- Constraint count matches Eva's LogUp share (`Q+T+1`); remaining ~3.5k constraints
  of the 1-MB Eva step are encode/Griffin (not ported yet).
- Next: FS challenge / `precommitted` split, then encode bit-length gadgets.
## Branch sync note

`spartan2-comparison` was fast-forwarded to include all `video` commits (lossless edit,
Spike A, still-image ingest) before Phase 2.0 work landed.
