# Combining Eva with C2PA: proposal and camera co-design roadmap

Goal: make an edited video's C2PA manifest carry a **cryptographic proof** that the declared edit
(e.g. brightness, crop) is the *only* change relative to the originally captured, signed content —
instead of the editing tool merely asserting it. This is a forward-looking proposal, not
implemented in this repo yet.

Background: [`c2pa_camera_architecture.md`](c2pa_camera_architecture.md) (C2PA/camera trust
model) and [`capture-signing-and-ingest.md`](capture-signing-and-ingest.md) (what Eva's proofs
actually bind to today).

## 1. Why C2PA and Eva are complementary, not competing

| | C2PA | Eva |
|--|------|-----|
| Proves | Who signed, and that bytes match a declared hash | That specific edited pixels are the result of applying a specific, bounded edit gadget to specific original pixels |
| Trust model | Certificate chain (X.509) to a claim signer; trust lists | Whatever key structure verifies the zk proof + signature (today: a custom Schnorr-style signature over BN254/Grumpkin in the Groth16 decider, see `video/src/decider.rs`) |
| Granularity | Whole asset or declared byte ranges (hard binding), or perceptual/derived content (soft binding) | Macroblock-level pixel witnesses (`orig_*_enc`), folded via IVC into two hashes (h1, h2) |
| Edit representation | `c2pa.actions` assertion — a **claim** like `"c2pa.color_adjustments"` | `EditGadget` — a **proof** that h2 pixels are the exact output of a specific gadget/config applied to h1 pixels |
| Weakness alone | Actions assertion is trusted, not verified against pixels | No standard container, trust-list, or distribution mechanism for the proof itself |

C2PA already has the container, the chain-of-custody model (via **ingredients**, see below), and
distribution/verification tooling (`c2pa-rs`, viewers, browser extensions). Eva already has the
zk machinery. Combining them means: **use C2PA as the envelope and trust/distribution layer;
use Eva as a new assertion type that upgrades one specific claim (the edit description) from
"trusted" to "proved."**

## 2. Mapping Eva concepts onto C2PA constructs

| Eva concept | C2PA construct | Notes |
|-------------|-----------------|-------|
| Original capture, hashed as h1 (`hash_recorder`, `hash_orig_macroblock`) | A **hard binding** assertion in the *capture* manifest, plus a new custom assertion (e.g. `org.eva.capture.v1`) carrying h1 in Eva's own hash domain (Griffin over macroblocks) | The camera's manifest must carry **both**: the standard BMFF hard binding (for existing C2PA verifiers) and the Eva-domain h1 (for Eva verifiers). They are hashes over different representations of the same capture — see [`c2pa_camera_architecture.md §2`](c2pa_camera_architecture.md#2-content-bindings-hard-vs-soft). |
| Edited asset referencing its source | **Ingredient** assertion (`c2pa.ingredient`) — C2PA's existing parent-asset relationship | The edited asset's manifest already declares "derived from capture X" via ingredient. Eva's proof needs exactly this link: it must prove edited pixels relate to **that specific** original's h1. |
| Edit description | `c2pa.actions` assertion (today: an unverified claim, e.g. `"c2pa.color_adjustments"`) | Keep this assertion for human-readable/legacy tooling, but treat it as **non-authoritative** once the proof assertion below is present. |
| Edit *proof* | New custom assertion, e.g. `org.eva.edit_proof.v1`, containing: gadget id (`Brightness`), compactified config, h1 (must match the ingredient's `org.eva.capture.v1` h1), h2, and the serialized zk proof (Nova IVC final state + Groth16 decider proof, or just the decider proof) | This is the assertion a C2PA verifier extension would additionally check by running Eva's verifier, not just trusting the claim signature. |
| Eva's own signature (`sigma` in `video/src/decider.rs`) | Redundant with, not a replacement for, the C2PA claim signature | The C2PA claim signature (COSE_Sign1, X.509 chain) already covers manifest integrity, including the new Eva assertion's bytes. Eva's internal decider signature over h1 can be kept as an extra binding *inside* the zk statement, or dropped in favor of relying on the outer COSE signature — an open design choice (see §5). |

### Why the ingredient link matters

C2PA's ingredient mechanism already expresses "this asset was derived from that one." Eva's IVC
state (`h1`) gives that same relationship **cryptographic teeth**: the edit-proof assertion's
`h1` value must equal the value asserted by the ingredient's own `org.eva.capture.v1` (or a
previous `org.eva.edit_proof.v1`, for chained edits) assertion. A verifier that checks both
(a) the ingredient hash-matches per standard C2PA rules and (b) the Eva h1 values match across
manifests gets a proved edit chain, not just an asserted one.

## 3. Proposed verification flow

```
Capture manifest (camera)
  c2pa.hash.bmff.v3            – standard hard binding (existing verifiers)
  org.eva.capture.v1           – h1 over orig_*_enc macroblocks (Eva verifiers)
  claim signature (COSE)       – camera's cert, signs the whole claim
        │
        │  ingredient reference
        ▼
Edited manifest (edit tool / app)
  c2pa.ingredient               – points at capture manifest
  c2pa.actions                  – "brightness adjustment" (human-readable, legacy)
  org.eva.edit_proof.v1         – gadget=Brightness, cfg=416, h1, h2, proof bytes
  claim signature (COSE)        – editing tool's cert, signs the whole claim
```

Verifier steps:

1. Standard C2PA validation: certificate trust, claim signature, hard binding hash match.
2. Ingredient check: edited manifest's ingredient hash matches the capture manifest.
3. **New:** Eva check — extract `org.eva.edit_proof.v1`; verify the zk proof; confirm its `h1`
   equals the capture manifest's `org.eva.capture.v1` value.
4. Report: "signed by X, derived from capture Y, and **cryptographically proved** to differ from
   Y only by a brightness adjustment with scale 416" — strictly stronger than today's C2PA-only
   claim, which stops after step 2 and trusts the `c2pa.actions` text.

## 4. The hard part: getting macroblocks and h1 at the camera

This is the actual crux, and it is a hardware/firmware problem, not a proof-system problem — see
[`capture-signing-and-ingest.md § Closing the gap`](capture-signing-and-ingest.md#closing-the-gap).
None of this works if `org.eva.capture.v1` is computed from a re-decoded MP4 on a phone/laptop
after the fact — that reintroduces exactly the trust gap Eva is meant to remove.

What camera co-design would need to provide, roughly in order of how deep the integration goes:

### Level 0 — software-only prototype (no manufacturer involvement)

- Trusted ingest app immediately reads the camera's output file, converts to Eva macroblocks
  (`yuv_to_macroblocks`), computes h1, and signs it as fast as possible after capture.
- Weakest guarantee: the gap between "shutter press" and "ingest app runs" is unprotected.
- Useful for validating the C2PA assertion schema and verifier tooling *before* asking any
  manufacturer for hardware changes.

### Level 1 — SDK-level hook (some manufacturer cooperation)

- Manufacturer's camera SDK (e.g. an existing RAW/ProRes RAW export API, or a proprietary
  capture SDK many manufacturers already ship to app partners) exposes a callback with
  **decoded YUV frames or macroblocks before final container encode**, still inside the
  vendor's trusted process.
- The Eva hashing routine runs inside that trusted process (or immediately after, in a
  library the manufacturer links in), signs h1 with a device key already used for existing
  C2PA signing (if the camera already does Level 0/1 C2PA signing, e.g. Leica-style), and
  writes `org.eva.capture.v1` into the manifest at the same point the hard binding is written.
- No new silicon required — this is firmware/SDK integration work, and is the most realistic
  near-term ask for cooperating manufacturers.

### Level 2 — dedicated hash engine, Eva-aware

- A fixed-function hash block (already common for the BMFF hard binding, see
  [`c2pa_camera_architecture.md §4`](c2pa_camera_architecture.md#4-where-signing-can-happen-and-why-it-matters-for-eva))
  is extended (or a second instance added) to also compute Griffin-style hashes over the
  macroblock domain, or at minimum to expose a raw macroblock tap that a small trusted
  co-processor hashes with Eva's algorithm.
- Requires manufacturer silicon/firmware changes; realistic only as a partnership, not a
  drop-in software update.
- This is the point where "co-design" in the literal sense (joint spec work with a silicon or
  camera-module vendor) starts to be necessary.

### Level 3 — standardized, multi-vendor

- A published assertion schema (see §6) and a reference hashing/proving library (this repo,
  packaged) that multiple manufacturers implement against, similar to how multiple vendors
  already implement C2PA's BMFF hashing independently.
- Long-term goal; not a prerequisite for Level 0/1 prototypes.

## 5. Open design questions to resolve before/while co-designing

1. **Which hash domain does the camera commit to?** Raw sensor YUV before ISP tone-mapping, or
   post-ISP YUV (what Eva macroblocks assume today)? Post-ISP is more practical (matches existing
   pipelines) but means ISP processing is *not* covered by the proof — needs to be stated
   explicitly in the assertion (e.g. a `pipeline_stage` field).
2. **Griffin hash in hardware or software?** Griffin (used by Eva for in-circuit efficiency) is
   unusual outside zk contexts; a hash engine built for SHA-256 (for the existing BMFF binding)
   will not natively compute it. Options: dedicated Griffin block (Level 2+), or software Griffin
   inside a trusted enclave/TEE at Level 1 without new fixed-function silicon.
3. **Key management.** Reuse the camera's existing C2PA device certificate/key for
   `org.eva.capture.v1`, or use a separate Eva-specific key registered in the same certificate
   (as an extension) or a parallel trust list? Reusing the existing C2PA key is simpler and
   avoids a second PKI.
4. **Proof size and where it lives.** Nova IVC state is small, but a Groth16 decider proof plus
   verifying key is not free; large videos may need many IVC steps. Decide whether the full proof
   is embedded in the manifest (self-contained, larger files) or referenced remotely (C2PA
   supports remote manifests) with only h1/h2/proof-hash embedded.
5. **Chained edits.** If an asset is edited twice (crop, then brightness), does each edit produce
   its own `org.eva.edit_proof.v1` referencing the previous one as ingredient (composable, matches
   C2PA's existing multi-hop ingredient model), or does Eva fold multiple edits into one proof
   over the original capture (smaller final proof, but requires re-proving from scratch if edits
   happen in different tools/sessions)? IVC folding naturally supports the composable case.
6. **Partial/streaming verification.** C2PA already supports segment-level hashing for large video.
   Eva's IVC steps are naturally chunked (`blocks_per_step`) — worth aligning IVC step boundaries
   with C2PA's segment boundaries so a verifier can check a prefix of a long video without the
   whole proof.

## 6. Concrete next steps

1. **Define the assertion schema** (`org.eva.capture.v1`, `org.eva.edit_proof.v1`) as JSON/CBOR:
   hash algorithm id (Griffin + parameters), curve, h1/h2 field element encoding, gadget id +
   compactified config encoding, proof format/version, pipeline-stage field (see §5.1).
2. **Build a Level 0 prototype** using `c2pa-rs` (the official Rust C2PA SDK — convenient since
   Eva is already Rust) to read/write manifests with the new assertion types, wired to today's
   `yuv_to_macroblocks` + `hash_recorder` + `edit_bright_only` / `edit_lossless_decider` examples.
   This validates the schema and verifier logic without needing any manufacturer involvement.
3. **Write an Eva-aware C2PA verifier extension**: given a manifest pair (capture + edited),
   extract the new assertions, run Eva's Nova/Groth16 verifier, and cross-check h1 against the
   ingredient chain (§3, step 3).
4. **Approach manufacturers already engaged with C2PA** (e.g. Leica, or camera-module vendors
   supplying phone OEMs) for a Level 1 conversation: ask specifically for a callback/SDK hook
   exposing pre-encode YUV/macroblocks inside their existing trusted signing process, rather than
   asking for new silicon up front. Bring the Level 0 prototype as a concrete integration target.
5. **Only after Level 1 is validated**, scope Level 2 hardware work (dedicated Griffin-capable
   hash engine) with a silicon/camera-module partner, informed by real proof-size and
   performance numbers from the prototype.

## 7. Related code and docs

| Piece | Role |
|-------|------|
| [`capture-signing-and-ingest.md`](capture-signing-and-ingest.md) | What Eva's h1/h2 and decider signature bind to today; the ingest trust gap this proposal aims to close |
| [`c2pa_camera_architecture.md`](c2pa_camera_architecture.md) | C2PA/camera background: bindings, trust boundary, signing points |
| [`hash_recorder`](../video/examples/hash_recorder.rs), [`hash_orig_macroblock`](../video/src/edit_only.rs) | Reference h1 computation Level 0/1 prototypes would reuse |
| [`video/src/decider.rs`](../video/src/decider.rs) | Eva's current signature (`sigma`) over h1 — candidate to keep, replace, or drop per §5.3 |
| [`docs/edit-only/README.md`](edit-only/README.md) | Lossless edit-only proof path (h2 over edited pixels + config) — the proof `org.eva.edit_proof.v1` would carry |
