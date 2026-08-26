# Offline decider vs ETH wrap — and WASM Spartan

**Repo:** `eva-miha`  
**Date:** 2026-08-20  
**Status:** Implemented for the WASM / editor path (2026-08-20). ETH Groth16 remains available natively via `DECIDER=groth16`. The in-tab wrap no longer builds `DeciderEthCircuit`.  
**Audience:** anyone wiring browser prove (`wasm-prove-spike`, fightfake.ai `/editor`)

This writes down why `DeciderEthCircuit` is EVM-shaped, what “switch away from EVM-friendly” would actually change, and whether that would make **Spartan wrap in wasm** realistic.

Related: [`SPIKE_A.md`](./SPIKE_A.md) (Groth16 PK OOM in wasm), [`spartan2-comparison/OPTION_C_TRANSPARENT.md`](./spartan2-comparison/OPTION_C_TRANSPARENT.md) (same ETH circuit, two SNARK backends), [`spartan2-comparison/results/phase3_option_c.md`](./spartan2-comparison/results/phase3_option_c.md) (measured sizes).

---

## 1. Two sizes that get mixed up

| Quantity | What it is | Does wrap grow with it? |
|----------|------------|-------------------------|
| **Steps** (e.g. 2 vs 600 folds) | How many times Nova folds. A 960×640 still at 4 MBs/step is **600** steps. | **No.** IVC already compressed the chain into one folded instance. |
| **Work per step** (e.g. 4 vs 16 MBs/fold) | Width of `EditOnlyCircuit`. | **Yes, somewhat.** The wrap re-checks that step R1CS. Fatter steps → fatter wrap. |
| Offline wrap (this work) | `Nova::verify` then Spartan on `NativePrimaryCircuit` | Dominated by **step** R1CS size (~86k QUICK), not by photo size. |

Do **not** grow `blocks_per_step` hoping wrap gets easier. More pixels per fold make the **step** circuit larger; the offline Spartan re-checks that larger primary R1CS. Photo size (4 MBs vs 2400 MBs) is almost all **step count** (Nova IVC, not wrap size).

```
960×640 photo  →  2400 MBs  →  600 Nova steps × 4 MBs   ← runs in-tab (WASM)
                                         │
                                         ▼  one folded instance
                         ┌───────────────┴────────────────┐
                         │  Offline wrap (editor, 2026-08-20)
                         │  Nova::verify — CycleFold in Grumpkin (CPU)
                         │  Spartan(NativePrimaryCircuit) — primary R1CS, FpVar
                         └────────────────────────────────┘

Historical (not used in-tab):
                              DeciderEthCircuit (~7.0M constraints)
                                         │
                              Groth16 or Spartan on that circuit  ← OOM in wasm
```

Nova in the editor is **edit-only**: `EditOnlyCircuit` + one gadget (redact or brightness), Griffin hashes of YUV 16×16 blocks, **no H.264 encode**. The wrap is not another edit step; it is compression of the finished Nova run.

---

## 2. What we measured

### Native QUICK (`BLOCKS_PER_STEP=4`, a few steps)

From [`phase3_option_c.md`](./spartan2-comparison/results/phase3_option_c.md):

| Piece | Size / time |
|-------|-------------|
| Nova step (`EditOnlyCircuit` + Brightness) | **~86,472** primary constraints; IVC ~0.8 s for 4 steps |
| `DeciderEthCircuit` | **~7.0M** constraints (Spartan pad **8.4M**) |
| Groth16 wrap | setup ~116 s, prove ~113 s, verify ~4 ms, proof **256 B**, PK **~1.3 GiB** |
| Spartan wrap (same circuit) | setup ~23 ms, prove ~79 s (synth 13 s + convert 59 s + core 7 s), verify ~3.7 s, proof **~370 KB** |

Spartan vs Groth16 only changes the **SNARK**. The statement is still `DeciderEthCircuit`.

### WASM

| Stage | Result |
|-------|--------|
| Nova preprocess + IVC (even 960×640, 600 folds) | **Works** in a browser worker (`wasm-prove-spike` `nova_start` / `nova_step`) |
| Groth16 setup or loading the 1.3 GiB PK | **`RuntimeError: unreachable`** / `alloc::rust_oom` ([SPIKE_A](./SPIKE_A.md)) |
| `SpartanDecider::prove(DeciderEthCircuit)` | Same trap. Node wasm, **32×32, 1 fold**, died after ~4 min in `generate_constraints` / convert. Browser 600-fold run: Nova JSON `wrap_error: "unreachable"`, `proof_bytes_len: 0`, `verified: false`. |

Wasm32 linear memory is not Node’s heap (`NODE_OPTIONS` does not help). Release traps compile OOM to `unreachable`.

---

## 3. “Eth” is not just a name

`DeciderEthCircuit` is the **EVM-shaped verifier** of a finished Nova run. The source says so:

```247:249:video/src/decider/mod.rs
/// Circuit that implements the in-circuit checks needed for the onchain (Ethereum's EVM)
/// verification.
pub struct DeciderEthCircuit<C1, GC1, C2, GC2>
```

```307:310:video/src/decider/mod.rs
        CS1: CommitmentScheme<C1, ProverParams = PedersenParams<C1>>,
        // enforce that the CS2 is Pedersen commitment scheme, since we're at Ethereum's EVM decider
        CS2: CommitmentScheme<C2, ProverParams = PedersenParams<C2>>,
```

Spartan in Eva proves **that same circuit** (`video/src/decider/spartan.rs`). Switching Groth16 → Spartan without changing the statement keeps the ETH encoding cost and drops the 256-byte / pairing benefits.

### 3.1 BN254 so Groth16 “hits” EVM pairing precompiles

The EVM has three contracts for one pairing-friendly curve (`alt_bn128` / BN254):

| Address | Name | Role |
|---------|------|------|
| `0x06` | `ecAdd` | G1 add |
| `0x07` | `ecMul` | G1 scalar mul |
| `0x08` | `ecPairing` | \(e(A_1,B_1)\cdots=1\) |

Groth16 verify is a short pairing product. **“Hits”** means the Solidity verifier `CALL`s `0x08` (plus MSM via `0x06`/`0x07`). It does not implement a Miller loop in Solidity.

Eva’s wrap is Groth16 on `Bn254`:

```715:716:video/src/decider/mod.rs
        let snark_proof = ark_groth16::Groth16::<E>::prove(&snark_pk, circuit, &mut rng)
```

```807:812:video/src/decider/mod.rs
        let snark_v = ark_groth16::Groth16::<E>::verify_proof_with_prepared_inputs(
            &snark_vk,
            &(proof, vec![UU.cmQ.into(), UU.cmW.into(), UU.cmE.into()]),
            &prepared_inputs,
        )
```

In a browser there is no `0x08`. arkworks still does the pairing in software, but the **circuit and curve** were chosen for L1.

Nova’s 2-cycle is BN254 + Grumpkin **because** `Fr(BN254) = Fq(Grumpkin)`. That cycle is an Ethereum convenience, not a general-purpose 2-cycle.

**If we do not care about EVM**

| Curve / system | Why |
|----------------|-----|
| **Pallas / Vesta (Pasta)** | True 2-cycle; CycleFold gadgets can be **native** `FpVar`. Halo2/IPA-style. No pairing, no Groth16 key. |
| **BLS12-381** | Better pairings than BN254 (Zcash, Ethereum *consensus*), still not cheap in EVM *execution*. |
| Stay BN254+Grumpkin but **stop encoding Grumpkin inside BN254 R1CS** | Possible if wrap is offline-only (see §5). |

### 3.2 Pedersen on CycleFold, and “EVM packing”

Nova commits to witnesses. Two usual schemes:

- **Pedersen:** \(C = rH + \sum w_i G_i\) — MSM. Transparent. Verify = more EC ops.
- **KZG:** polynomial commit. Verify = pairings. Needs an SRS.

Sonobe’s ETH decider uses **KZG on BN254 (CS1)** so those checks also hit `0x08`:

```190:196:folding-schemes/src/folding/nova/decider_eth.rs
        // we're at the Ethereum EVM case, so the CS1 is KZG commitments
        CS1::verify_with_challenge(
            &cs_vk,
            proof.kzg_challenges[0],
            &U.cmW,
            &proof.kzg_proofs[0],
        )?;
```

Eva’s `video` decider forces **Pedersen on C2 (Grumpkin CycleFold)** so the Groth16 circuit can *open* those commitments as EC gadgets (`PedersenGadget`), not pairings-inside-R1CS:

```632:646:video/src/decider/mod.rs
            PedersenGadget::<C2, GC2>::commit(...)?.enforce_equal(&cf_U_i.cmE)?;
            PedersenGadget::<C2, GC2>::commit(...)?.enforce_equal(&cf_U_i.cmW)?;
```

**EVM packing** is not a crypto primitive. Groth16 public IO is a vector of BN254 `Fr` elements. A Solidity verifier is `verify(proof, uint256[] memory input)`. Every public value — scalars and curve points — is flattened into that array. Eva does it in `Decider::verify`:

```769:801:video/src/decider/mod.rs
        let public_inputs = vec![
            vec![one, i.into_bigint()],
            z_0.iter().map(|i| i.into_bigint()).collect::<Vec<_>>(),
            /* h2, camera vk (x,y), U.u, */
            E::G1::normalize_batch(&[U.cmQ, U.cmW, U.cmE, u.cmQ, u.cmW, cmT])
                .iter()
                .flat_map(|i| { /* x,y chopped into NonNativeUintVar limbs */ })
            vec![r.into_bigint()],
        ]
        .concat();
```

Offline, a verifier can take structs `{ cmQ, cmW, cmE }` and a 370 KB file. It does not need this `uint256[]`.

### 3.3 Non-native Grumpkin inside BN254

CycleFold’s R1CS lives in **Grumpkin’s field**. The wrap SNARK lives in **BN254 `Fr`**. Different moduli → each Grumpkin integer is **limbs** of native `Fr`:

```175:199:folding-schemes/src/folding/circuits/nonnative/uint.rs
pub struct NonNativeUintVar<F: PrimeField>(pub Vec<LimbVar<F>>);
    pub const fn bits_per_limb() -> usize { /* ... */ 40 }
```

~254-bit Grumpkin element → ~7×40-bit limbs. A point is `(x,y)` twice that.

```13:20:folding-schemes/src/folding/circuits/nonnative/affine.rs
/// NonNativeAffineVar ... over the constraint field. It is not intended to perform operations,
/// but just to contain the affine coordinates in order to perform hash operations of the point.
pub struct NonNativeAffineVar<C: CurveGroup> {
    pub x: NonNativeUintVar<C::ScalarField>,
    pub y: NonNativeUintVar<C::ScalarField>,
}
```

The wrap allocates CycleFold R1CS as non-native and checks `Az∘Bz` with limb muls (`RelaxedR1CSGadget::check_nonnative` in `video/src/decider/mod.rs`). That is the bulk of the **~7M** constraints. Solidity never implements Grumpkin; Groth16 attests the limb equations, and L1 only checks BN254 pairings.

A Pasta 2-cycle wrap would use `FpVar` on **both** sides.

### 3.4 Public inputs laid out for a contract

`new_input` = public (verifier sees it). `new_witness` = private.

```400:445:video/src/decider/mod.rs
        let i = FpVar::new_input(...)?;     // fold count
        let z_0 = Vec::new_input(...)?;     // initial IVC state
        // h1 witness, h2 public
        let p = GC2::new_input(...)?;       // camera vk
        // U.u, U.cm{Q,W,E}, u.cm{Q,W}, cmT, r  — all new_input
```

A contract would take `(i, z_0, h2, vk, commitments, r)` as calldata, rebuild the packed vector, call `0x08`. `h1` is a witness: the chain is expected to know the **edited** hash; the original is bound by σ inside the circuit.

### 3.5 Groth16-shaped IO vs a ~370 KB Spartan blob

| | Groth16 wrap (ETH) | Spartan wrap (offline) |
|--|--------------------|------------------------|
| Proof | ~256 B | ~370 KB (QUICK) |
| Verify | pairing + small MSM, ~ms / ~150k gas | seconds of hashes / field ops |
| Public IO | must stay a short `uint256[]` | can be a file |

Groth16 verifier cost grows with public-input length (`gamma_abc_g1` MSM). That is why points are packed as limbs. Spartan does not need 256 B, but **running Spartan on `DeciderEthCircuit`** keeps the fat ETH layout **and** the fat proof.

### 3.6 Camera σ inside the wrap

Schnorr-like σ on Grumpkin over `h1` (`wasm-prove-spike/src/browser.rs` `device_sigma`). Checked **again** in-circuit so one Groth16 implies “Nova valid **and** this camera signed `h1`”:

```486:491:video/src/decider/mod.rs
            let e = CRHGadget::evaluate(&crh_params, &[sigma_r.clone(), px, py, z_i[0].clone()])?;
            (g.scalar_mul_le(sigma_s.iter())? - p.scalar_mul_le(e.iter())?)
                .to_constraint_field()?[0]
                .enforce_equal(&sigma_r)?;
```

On-chain that fusion is the point (no extra Grumpkin verifier in Solidity). Offline, Schnorr is microseconds in ordinary code. Wrapping only “Nova folded correctly” and checking σ **outside** drops those gadgets (small vs 7M, but honest separation).

---

## 4. Would switching away from EVM-friendly help?

**Yes, for wrap size. No, if “switch” only means Spartan instead of Groth16 on the same circuit.**

```
What we did in the editor (2026-08-20)
  Nova IVC in wasm                 ✓
  Nova::verify (native CycleFold)  ✓  wrap_offline in browser.rs
  Spartan(NativePrimaryCircuit)    ✓  ~step R1CS, no DeciderEthCircuit
  Spartan(DeciderEthCircuit)       ✗  removed from nova_finish (~7M OOM)
```

Measured ETH wrap vs the new statement:

| Statement | Constraints (order) | WASM Spartan? |
|-----------|---------------------|----------------|
| Offline wrap: `Nova::verify` + Spartan(`NativePrimaryCircuit`) | **~step circuit** (QUICK ~86k, not 7M) | **This is `nova_finish`** |
| `DeciderEthCircuit` (historical in-tab attempt) | **~7M** (pad 8.4M) | **No** (OOM at synthesize/convert) |
| Same Nova, σ outside, still ETH circuit | still ~7M | No |
| Pasta Nova + slim CCS wrap | design-dependent | Not started |

Why 7M is not “86k × something from 600 steps”: the wrap checks **one** folded instance. The 86k is the **step** R1CS stored as matrices inside the decider; CycleFold non-native check of those matrices is what explodes.

Spartan’s native 79 s on `DeciderEthCircuit` was mostly **convert** (59 s) of 8.4M padded rows. The offline wrap pads **131,072** rows and converts in wasm in a few seconds.

**Verdict:** the in-tab wrap **left the EVM encoding**. CycleFold is `Nova::verify` on the CPU. Spartan is `NativePrimaryCircuit` only. If wasm Spartan still traps, `ivc_verified` can still be true (`proof_system: nova-ivc`). Pasta remains optional (CycleFold inside a SNARK without limbs).

---

## 5. How to switch (phased)

### Phase 0 — product honesty (done in fightfake.ai editor)

- Tab = Nova IVC prover (`nova_start` / `nova_step` / `nova_digest`).
- If `nova_finish` traps: `proof_system: nova-wasm`, `complete: false`, `succinct: false`, `wrap_error`.
- Do not label that JSON “Nova + Spartan.”

### Phase 1 — wrap natively, keep BN254 Nova (smallest engineering)

Nova stays as today (BN254 + Grumpkin, `EditOnlyCircuit`). After IVC:

1. Serialize the folded instance (or keep it in a native helper).
2. Run `SpartanDecider::prove(DeciderEthCircuit::from_nova(...))` **on the host**, not in wasm.
3. Browser downloads / displays the 370 KB blob; verify native or in a non-wasm tool.

This does **not** shrink the circuit. It **does** make wrap possible (Spike A native already does Groth16; phase3 already does Spartan). WASM never allocates 7M constraints.

Ship path for `/editor` if we want a real succinct file soon.

### Phase 2 — implemented (2026-08-20)

The in-tab wrap **no longer synthesizes `DeciderEthCircuit`**. See §8.

CycleFold is attested by **`Nova::verify`** (native Grumpkin R1CS), not by a primary-only SNARK pretending to be the whole IVC. The SNARK is only the primary relaxed R1CS (`NativePrimaryCircuit`).

### Phase 3 — retarget Nova to Pasta (not done)

Port `EditOnlyCircuit`, Griffin, gadgets, and Nova to Pallas/Vesta. CycleFold gadgets would be native *inside a SNARK* as well. Largest port; not required once CycleFold is checked on CPU.

Cost: field/gadget rewrite across `video` + `folding-schemes`. Only needed if we want a *single* SNARK that includes CycleFold without limbs.

### Explicit non-goals

- Growing `WASM_BPS_CAP` / MBs per step to “help wrap.”
- Loading the 1.3 GiB Groth16 PK into wasm ([SPIKE_A](./SPIKE_A.md)).
- Spartan-on-`DeciderEthCircuit` in the browser.

---

## 6. Would WASM Spartan become possible?

**Yes, for the offline statement.** Measured 2026-08-20 in Node `wasm32` (same linear-memory model as a tab):

| Test | Constraints | Proof | Wall | Result |
|------|-------------|-------|------|--------|
| `spartan_tiny_smoke` (`a*b=c`) | 2 (pad 2) | — | 0.5 s | **verified** |
| 16×16, 1 MB, 1 fold, `bps=1` | **113,479** (pad 131,072) | ~293 KB | ~15 s total, wrap ~7 s | **`nova-offline-spartan`** |
| 32×32, 4 MBs, 1 fold, `bps=4` (editor step width) | **129,439** (pad 131,072) | ~294 KB | ~17 s total | **`nova-offline-spartan`** |

Replay:

```bash
cd wasm-prove-spike
wasm-pack build --target nodejs --out-dir pkg-node --release --features wasm-js
node run-wasm-spartan.js
```

A 960×640 still is **600 folds at `bps=4`**. Wrap size follows the **step** R1CS, not the fold count, so the Spartan statement should stay ~129k constraints. Nova IVC time still grows with steps.

| Approach | In-tab Nova | In-tab wrap | Status |
|----------|-------------|-------------|--------|
| Spartan(`DeciderEthCircuit`) | Yes | **No** (OOM) | Removed from `nova_finish` |
| Phase 1: host wrap | Yes | Host | Still valid for Groth16 256 B |
| Phase 2: `Nova::verify` + Spartan(primary) | Yes | **Yes (measured)** | ~129k cons at editor `bps=4`, not 7M |
| Phase 3: Pasta Nova | After port | Unknown | Not started |

---

## 7. Code map (current)

| Path | Role |
|------|------|
| `video/src/edit_only.rs` | `EditOnlyCircuit` — what WASM Nova proves |
| `video/src/decider/offline.rs` | `NativePrimaryCircuit` — in-tab Spartan statement |
| `video/src/decider/mod.rs` | Groth16 `DeciderEthCircuit` (native EVM path only) + `check_native` |
| `video/src/decider/spartan.rs` | Generic `SpartanDecider::prove(ConstraintSynthesizer)` |
| `folding-schemes/.../nonnative/` | Limb encoding — **not** used by `NativePrimaryCircuit` |
| `wasm-prove-spike/src/browser.rs` | In-tab session; `wrap_offline` |
| `wasm-prove-spike/src/eth_spike.rs` | Native Spike A Groth16; **not compiled for wasm32** |

---

## 8. What was implemented (code references)

### Statement change

| Old (`nova_finish`) | New |
|---------------------|-----|
| `DeciderEthCircuit::from_nova` → `SpartanDecider::prove` (~7M cons, non-native CF + σ) | `Nova::verify` (native primary + CycleFold) then `SpartanDecider::prove(NativePrimaryCircuit)` |

CycleFold is **not** dropped. It is checked in `Nova::verify`:

```1080:1123:folding-schemes/src/folding/nova/mod.rs
    fn verify(...) {
        ...
        vp.r1cs.check_current_instance_relation(&w_i, &u_i)?;
        vp.r1cs.check_relaxed_running_instance_relation(&W_i, &U_i, E)?;
        vp.cf_r1cs.check_relaxed_cyclefold_instance_relation(&cf_W_i, &cf_U_i, cf_E)?;
        Ok(())
    }
```

That `cf_r1cs` check is **Grumpkin-native** matrix algebra, not `NonNativeUintVar` inside BN254.

### New files / modules

| Path | What it does |
|------|----------------|
| [`video/src/decider/offline.rs`](../video/src/decider/offline.rs) | `NativePrimaryCircuit` (`RelaxedR1CSGadget::check_native` only), `native_primary_from_running`, `prove_native_primary` / `verify_native_primary` |
| [`video/src/decider/mod.rs`](../video/src/decider/mod.rs) | `pub mod offline`; module docs distinguish ETH vs offline |
| [`wasm-prove-spike/src/lib.rs`](../wasm-prove-spike/src/lib.rs) | wasm32: only `browser` + `nova_*` JS exports. `eth_spike` / Groth16 **not compiled** |
| [`wasm-prove-spike/src/eth_spike.rs`](../wasm-prove-spike/src/eth_spike.rs) | Native Spike A only (`DeciderEthCircuit` + Groth16 FFPB) |
| [`wasm-prove-spike/src/browser.rs`](../wasm-prove-spike/src/browser.rs) | `verify_device_sigma`, `wrap_offline`, `nova_finish` — **zero** `DeciderEthCircuit` |
| [`video/examples/edit_lossless_decider.rs`](../video/examples/edit_lossless_decider.rs) | `DECIDER=offline` (returns before `from_nova`) |
| [`fightfake.ai-web/src/components/editor/EditorWorkspace.tsx`](../../fightfake.ai-web/src/components/editor/EditorWorkspace.tsx) | JSON `proof_system` / `complete` / `succinct` / notes |
| [`fightfake.ai-web/src/lib/editor-prove.ts`](../../fightfake.ai-web/src/lib/editor-prove.ts) | Progress label for the offline wrap |
| [`fightfake.ai-web/public/editor-prove/worker.js`](../../fightfake.ai-web/public/editor-prove/worker.js) | Calls `nova_finish`; trap fallback keeps `ivc_verified` |

### Native primary circuit

```34:65:video/src/decider/offline.rs
pub struct NativePrimaryCircuit<F: PrimeField> { r1cs, z, u, e }
impl ConstraintSynthesizer<F> for NativePrimaryCircuit<F> {
    fn generate_constraints(...) {
        // R1CSVar + FpVar only → RelaxedR1CSGadget::check_native
    }
}
```

The native check (no limbs) is:

```85:99:video/src/decider/mod.rs
    pub fn check_native<F: PrimeField>(...) {
        // Az ∘ Bz == u Cz + E over FpVar
    }
```

Public IO: `u` and the running instance `x` (`new_input`). Witness: `QW` and `E`. No affine limbs, no PedersenGadget on C2, no Schnorr gadget.

Spartan backend is unchanged and generic:

```231:239:video/src/decider/spartan.rs
impl SpartanDecider {
    pub fn prove<C, F>(circuit: C) -> Result<(SpartanProof, SpartanVerifierKey), Error>
    where
        C: ConstraintSynthesizer<F>,
```

### Browser wrap

```421:465:wasm-prove-spike/src/browser.rs
fn wrap_offline<FC: FCircuit<Fr>>(...) {
    // 1. Schnorr σ in ordinary code (verify_device_sigma)
    // 2. NovaFC::verify — CycleFold included
    // 3. prove_native_primary(NativePrimaryCircuit)
}
```

wasm32 does not compile the Groth16 path:

```25:40:wasm-prove-spike/src/lib.rs
#[cfg(not(target_arch = "wasm32"))]
mod params;
#[cfg(not(target_arch = "wasm32"))]
mod eth_spike; // DeciderEthCircuit + Groth16 Spike A
```

`SessionDone` now has `ivc_verified` and `proof_system`: `nova-offline-spartan` (both ok) or `nova-ivc` (IVC ok, Spartan failed). `nova_finish` keeps `FFSP1` (`nova_take_proof`) and `FFIV1` (`nova_take_ivc`). The wrap VK is hashed (`vk_sha256`) and published once per `circuit_id`. `/verify` calls `nova_verify_full`: reconstruct `vp` from gadget+bps, `Nova::verify` (CycleFold included), camera σ, Spartan, bind public IO to `U.u` / `U.x`.

### Checkable artifact (newsroom sidecar)

| Piece | Where | Size |
|-------|--------|------|
| Claim + `FFIV1` Nova transcript + `FFSP1` Spartan | `{stem}-edit.ffproof.json` from `/editor` | transcript is the witnesses (MB-scale); wrap ~300 KiB |
| Spartan VK (`FFSV1`) | `/verify/vk/{circuit_id}.bin` | linear in wrap R1CS; **shared** |
| Full check | `/verify` runs `Nova::verify` (CycleFold included) + camera σ + Spartan | verifier reconstructs `vp` from gadget+bps |

```bash
# Well-known VKs (gadget + bps=4 dummy 64×16, no 600-fold prove)
cargo run --release -p wasm-prove-spike --bin export-offline-vk -- \
  ../fightfake.ai-web/public/verify/vk
```

### How to run

```bash
# Unit tests (tiny native-field Spartan, not the 7M circuit)
cargo test -p video --features cpu,spartan --lib decider::offline --release

# Full lossless example, no ETH circuit
export DATA_PATH=/path/to/data_parsed
QUICK=1 DECIDER=offline cargo run --release -p video --example edit_lossless_decider

# Browser: rebuild wasm
cd wasm-prove-spike && wasm-pack build --target web --release --features wasm-js
# copy pkg/wasm_prove_spike.js, .d.ts, _bg.wasm into fightfake.ai-web/public/editor-prove/
# do not overwrite worker.js

# JS exports: nova_start / nova_step / nova_digest / nova_finish /
# nova_take_proof / nova_take_ivc / nova_verify_full / spartan_verify
# (no spike_prove_* Groth16 API).
```

### What we did **not** do

- Delete Groth16 / `DeciderEthCircuit` (native `DECIDER=groth16` still exists for anyone who wants L1 later).
- Port Nova to Pasta. Not required for native CycleFold *verify*.
- Put CycleFold *inside* the Spartan / Groth16. That is the **old EVM wrap** (`DeciderEthCircuit`, ~7M cons, non-native Grumpkin limbs) so a pairing-only chain can check the 2-cycle. The **new** offline decider already checks CycleFold: `Nova::verify` runs `cf_r1cs` over Grumpkin. Full third-party verify is “export the IVC transcript (`FFIV1`) and run that same `Nova::verify`,” not “stuff CycleFold back into the SNARK.” Spartan only wraps the primary folded instance.
