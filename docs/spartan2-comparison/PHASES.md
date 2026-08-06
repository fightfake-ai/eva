# Phased plan

Each phase has explicit entry/exit criteria. Update the checkboxes in
[README.md](./README.md) as work completes.

---

## Phase 0 — Infrastructure & R1CS characterization

**Goal:** Measure Eva's circuit shape and prove Spartan2 builds in this repo.

**Entry:** `cpu-backend` merged; branch `spartan2-comparison` created.

**Tasks:**

- [x] Documentation scaffold (`docs/spartan2-comparison/`)
- [x] `comparison` workspace crate
- [x] `r1cs_stats` example — synthesize Eva circuits, print R1CS dimensions
- [x] `phase0_baseline` example — NeutronNova smoke test on BN254
- [x] Record baseline numbers in `docs/spartan2-comparison/results/phase0.md`

**Exit criteria:**

- Eva augmented-circuit constraint count documented for `BLOCKS_PER_STEP=256`
- NeutronNova `setup → prep_prove → prove → verify` succeeds on BN254
- No regressions to existing `folding-schemes` / `video` crates

**Run:**

```bash
BLOCKS_PER_STEP=256 cargo run --release -p comparison --example r1cs_stats
NUM_STEPS=4 cargo run --release -p comparison --example phase0_baseline
```

---

## Phase 1 — Matched-size NeutronNova benchmark

**Goal:** Compare prove time at similar R1CS scale, without lookup arguments.

**Tasks:**

- [x] Implement bellpepper `PlaceholderStepCircuit` sized to match Eva step R1CS constraint count
- [x] Nova baseline: time `AugmentedFCircuit` synthesis + `compute_cmT` + `prove_step`
- [x] NeutronNova baseline: time `prep_prove + prove` with `MATCH_EVA=1` constraint matching
- [x] `phase1_benchmark` example + initial results in `results/phase1.md`
- [x] `phase1_scale` multi-scale sweep (blocks 4, 16, 64)
- [x] Full-scale Nova run at BLOCKS_PER_STEP=256 (~1.43M constraints)
- [x] Peak RSS for Nova full-scale (~6.2 GB)
- [x] Full-scale NeutronNova at BLOCKS_PER_STEP=256 (~5.1 s/step, ~6.7 GB RSS)

**Exit criteria:**

- Published table: wall time, peak RSS, constraints, steps for both backends
- Decision point: >2× improvement on target hardware → proceed to Phase 2

---

## Phase 2 — Lookup arguments

**Goal:** Port or re-implement Eva's lookup argument in a NeutronNova-compatible form.

**Tasks:**

- [x] Document lookup witness layout (see [LOOKUPS.md](./LOOKUPS.md))
- [x] Document Eva lookup protocol (`folding-schemes/src/frontend/`, `AugmentedFCircuit` integration)
- [x] Prototype Option A LogUp in bellpepper (1-MB pixels + Q=2320, NeutronNova smoke)
- [ ] Evaluate Spartan2 / Lasso zero-check reduction vs custom lookup
- [x] Scale to Q≈2320 (full 1-MB NoOp committed queries) + record timings
- [ ] FS challenge binding (`num_challenges` / Poseidon(cmQ) analogue)
- [ ] Port encode bit-length gadgets; full Eva step as `SpartanCircuit`

**Exit criteria:**

- Step circuit with lookups proves under NeutronNova
- Functional equivalence test against Nova step on same inputs

**Run:**

```bash
NUM_QUERIES=384 cargo run --release -p comparison --example phase2_lookup_smoke
cargo test -p comparison bellpepper --release
```

**Results:** [`results/phase2.md`](./results/phase2.md)
---

## Phase 3 — Full video pipeline

**Goal:** End-to-end comparison on real `data_parsed` video data.

**Tasks:**

- [ ] Replace Nova `prove_step` loop with NeutronNova batch prover
- [ ] Replace / compare Groth16 decider vs Spartan final proof
- [ ] Measure full-video prove time, peak memory, proof size
- [ ] Document verifier cost (Ethereum gas if applicable)

**Exit criteria:**

- Same video authenticated under both stacks (functional test)
- Written recommendation: stay on Nova, migrate, or hybrid

---

## Phase 4 — Production decision (optional)

Only if Phase 3 shows clear wins:

- [ ] Remove or feature-gate unused Nova/CycleFold paths
- [ ] CI benchmarks on fixed hardware
- [ ] PR to `video` branch with `--features neutronnova` or full switch

---

## Risk register

| Risk | Mitigation |
|------|------------|
| Bellpepper re-write takes months | Phase 1 uses size-matched placeholder first |
| Lookups cannot map to zero-check cheaply | Evaluate hybrid: Nova step + Spartan decider only |
| NeutronNova batch model doesn't fit streaming video | Use `prep_prove` per chunk; study Vega paper streaming patterns |
| BN254 Hyrax PCS slower than Pedersen for Eva witness shape | Benchmark `is_small=true` path; try `is_small=false` |
