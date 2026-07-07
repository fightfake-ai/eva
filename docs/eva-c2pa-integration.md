# Combining Eva with C2PA: architecture and integration

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
| Trust model | Certificate chain (X.509) to a claim signer; trust lists | zk proof verified by whoever runs Eva's verifier; in the full pipeline, a Schnorr-style signature over BN254/Grumpkin inside the Groth16 decider (see §2) |
| Granularity | Whole asset or declared byte ranges (hard binding), or perceptual/derived content (soft binding) | Macroblock-level pixel witnesses (`orig_*_enc`), folded via IVC into two field elements (h1, h2) |
| Edit representation | `c2pa.actions` assertion — a human-readable **claim** like `"c2pa.color_adjustments"` | `EditGadget` — a **proof** that h2 pixels are the exact output of a specific gadget/config applied to h1 pixels |
| Weakness alone | Actions assertion is trusted, not verified against pixels | No standard container, trust-list, or distribution mechanism for the proof itself |

C2PA already has the container, the chain-of-custody model (via **ingredients**, see below), and
distribution/verification tooling (`c2pa-rs`, viewers, browser extensions). Eva already has the
zk machinery. Combining them means: **use C2PA as the envelope and trust/distribution layer;
use Eva as a new assertion type that upgrades one specific claim (the edit description) from
"trusted" to "proved."**

## 2. What the Groth16 decider verifies

Eva's proof system has two layers.

**Layer 1 — Nova IVC** proves, step by step over each batch of macroblocks, that the edit
gadget was applied correctly and that the Griffin hash chain (h1, h2) was advanced correctly.
After `num_steps` steps, the IVC has produced two field elements `[h1, h2]` and a *running
instance* — a cryptographic accumulation (via Pedersen commitments) of all the step witnesses.

**Layer 2 — Groth16 decider** wraps the entire IVC argument in a single, compact Groth16 proof.
The Groth16 circuit takes the final running instance (U_i, u_i) and performs one last Nova
folding step *inside the circuit*, then checks whether the result satisfies the step circuit's
R1CS. That last check is the key: by Nova's security, if the folded instance satisfies the R1CS,
then every preceding folding step was also correct, and the entire IVC computation is valid.
So the Groth16 decider really is "just checking the final folded instance" — but that is not
a trivial check: the running instance is a commitment to the entire computation, and the R1CS
check certifies the whole chain.

On top of the R1CS check, the decider circuit also verifies two auxiliary facts:

**Final state consistency.** The IVC starts from `z_0 = [0, 0]` (declared as a public input to
the decider). After exactly `i` steps (also a public input), the output is `z_i = [h1, h2]`.
`z_i[1] = h2` is a public input; `z_i[0] = h1` is a witness (private). The circuit checks that
the committed running instance's public I/O is consistent with exactly these values: it was
initialised at `z_0`, ran for `i` steps, and produced `h2`. This rules out the prover supplying
an IVC chain that ran from a different starting point, or ran a different number of steps, or
claims a different h2.

**Signature verification.** `sigma = (sigma_r, sigma_s)` is a Schnorr-style signature over h1,
computed by the prover using the device's private key `sk`. The device's public key
`vk = G * sk` is a public input to the Groth16 circuit. Inside the circuit the Schnorr equation
is verified: `G * sigma_s - H(sigma_r, vk, h1) * vk = sigma_r` (see
`video/src/decider.rs` lines 399–474). Both `sigma` and `h1` are witnesses, so the verifier does
not learn h1 directly. What the valid proof guarantees is: "there exists an h1 such that (a) it
is the correct first component of the IVC final state, and (b) the device holding the private key
for `vk` signed it." In other words, `sigma` is indeed the signature over h1 — the device key
signs h1, and that signature is proved correct inside the Groth16 circuit, binding the zk
argument to the specific device that captured the footage.

**Without the Groth16 decider** you could verify the Nova IVC directly, but this requires
O(num_steps) elliptic-curve work for the verifier (one check per folding step), there is no
sigma check, and the argument is not constant-size. The Groth16 decider is what turns the IVC
into a fixed ~200-byte proof that is cheaply verifiable and can be embedded in a C2PA manifest.

## 3. Eva's edit proof: what is and is not proved

Eva's "edit-only" circuit (the name means the encoder is excluded, not that only a single step
runs) proves the following over the full video:

- h1 is the Griffin hash chain over the **original** macroblocks (`orig_*_enc`).
- h2 is the Griffin hash chain over the **edited** macroblocks (the same macroblock layout, with
  the edit gadget applied to each pixel).
- The edit gadget (e.g. `Brightness` with scale 416) was applied correctly to every macroblock.

What is **not** proved: the encoding of the edited macroblocks into a final H.264/HEVC video
file. After the proof is generated, the edited macroblocks are exported to planar YUV
(`native_edit_export_yuv`) and then encoded with a standard tool (e.g. ffmpeg). That encoding
step is outside the proof. The proof only covers the pixel-domain transformation: original
macroblocks → edited macroblocks. The subsequent re-encoding of edited macroblocks to a
deliverable video file is a trusted post-processing step.

This is a deliberate design choice: including the full H.264 encoder in the circuit would require
millions of constraints per frame just for entropy coding (CABAC), before any pixel operations —
not tractable at video resolutions today.

## 4. Mapping Eva concepts onto C2PA constructs

| Eva concept | C2PA construct | Notes |
|-------------|-----------------|-------|
| Original capture, hashed as h1 (`hash_recorder`, `hash_orig_macroblock`) | `c2pa.hash.bmff.v3` (standard hard binding) **plus** new assertion `org.eva.capture.v1` carrying h1 | Two separate assertions: BMFF for container tamper-evidence; Eva h1 for pixel-level proved edits. They hash different representations and are never numerically equal — see [`c2pa_camera_architecture.md §2`](c2pa_camera_architecture.md#2-content-bindings-hard-vs-soft). |
| Edited asset referencing its source | `c2pa.ingredient` — C2PA's existing parent-asset relationship | The edited manifest declares "derived from capture X." Eva's proof must reference the same h1: the ingredient's `org.eva.capture.v1` h1 must equal the edit proof's h1. |
| Edit description | `c2pa.actions` (e.g. `"c2pa.color_adjustments"`) | Keep for human-readable / legacy tooling; non-authoritative once the proof assertion is present. |
| Edit *proof* | New assertion `org.eva.edit_proof.v1`: gadget id, config, h1, h2, Groth16 proof bytes | An Eva-aware C2PA verifier runs Eva's Groth16 verifier on this, rather than trusting the claim signature alone. |
| Eva's decider signature (`sigma`) | Redundant with, but distinct from, the C2PA claim signature | The COSE_Sign1 signature already covers manifest integrity. Eva's sigma is a separate statement *inside the zk proof* about which device key signed h1. Both can coexist using the same camera device key. |

### Why the ingredient link matters

C2PA's ingredient mechanism expresses "this asset was derived from that one." Eva's h1 gives that
relationship cryptographic force: the edit-proof assertion's h1 must equal the ingredient's
`org.eva.capture.v1` h1. A verifier that checks both (a) standard C2PA ingredient chain and
(b) h1 values match across manifests gets a proved edit chain, not just an asserted one.

## 5. Proposed verification flow

```
Capture manifest (camera / drone)
  c2pa.hash.bmff.v3            – standard hard binding over container bytes
  org.eva.capture.v1           – h1 = Griffin hash over macroblock pixels
  claim signature (COSE)       – device cert; signs the whole claim
        │
        │  c2pa.ingredient reference
        ▼
Edited manifest (edit tool / workstation)
  c2pa.ingredient               – points at capture manifest
  c2pa.actions                  – "brightness adjustment" (human-readable)
  org.eva.edit_proof.v1         – gadget=Brightness, cfg=416, h1, h2, Groth16 proof
  claim signature (COSE)        – editing tool's cert; signs the whole claim
```

Verifier steps:

1. Standard C2PA validation: certificate trust chain, COSE signature, BMFF hard binding match.
2. Ingredient check: edited manifest's ingredient hash matches the capture manifest.
3. **Eva check:** extract `org.eva.edit_proof.v1`; run the Groth16 verifier; confirm the proof's
   h1 equals `org.eva.capture.v1` in the ingredient.
4. Report: "signed by device X, derived from capture Y, and cryptographically proved to differ
   from Y only by brightness with scale 416" — strictly stronger than the C2PA-only chain, which
   stops at step 2 and trusts the `c2pa.actions` text.

## 6. Getting macroblocks and h1 at capture

None of this works if `org.eva.capture.v1` is computed from a re-decoded recording after the fact
— that reintroduces the trust gap (see
[`capture-signing-and-ingest.md`](capture-signing-and-ingest.md)). The h1 must be computed from
pixel data before it passes through general-purpose software.

Integration depth is described in four levels below. The key question is not capability but trust
isolation: who computed the hash, and what could have touched the pixels before they were hashed?

### "Original recording" — what it is and how macroblocks are derived from it

When a camera stores a video file — typically H.264 or H.265 in an MP4/MOV container — that file
is the "original recording." It is the output of the on-device hardware encoder, produced from
ISP-processed YUV frames. The recording is a compressed bitstream; no pre-encode macroblocks in
Eva's witness format are written to storage.

To derive Eva's macroblock witnesses (`orig_*_enc`) from the recording at prove time:

1. **Decode** the compressed stream back to raw YUV frames using a decoder (e.g. ffmpeg):
   `H.264 NAL units → CABAC decode → iDCT → add prediction → YUV`.
2. **Tile** the planar YUV into Eva's layout using `yuv_to_macroblocks`: extract each 16×16 luma
   block and 8×8 chroma blocks in left→right, top→bottom order into `orig_y_enc`, `orig_u_enc`,
   `orig_v_enc`.

**This decode-then-tile step is unproved** — it is the ingest trust gap described in
`capture-signing-and-ingest.md`. It is the starting point for Level 0 and the gap that higher
levels narrow.

**Whether macroblocks need to be stored separately** depends on which level is used and what the
hash is computed over:

- **Level 0 / hash over decoded macroblocks:** h1 is computed over post-decode macroblocks, which
  are derived deterministically from the recording. The recording is the only thing that needs to
  be retained; witnesses are regenerated by decoding at prove time. No separate macroblock storage
  is needed.
- **Levels 1 and 2 / hash over pre-encode pixels:** if the hash is computed from the pixel stream
  before H.264 encoding (from the ISP/encoder bus), those exact pixels cannot be recovered by
  decoding the recording, because H.264 encoding is lossy. In this case, either a lossless
  recording format (ProRes, lossless H.264) must be used so decoded frames match pre-encode
  frames, or the raw pre-encode macroblocks must be stored alongside the recording (~30–100MB/min
  at 1080p after compression — comparable to the recording itself). This is an open design
  decision for Level 1/2 deployments.

| Phase | What exists |
|-------|-------------|
| At capture | The original recording (MP4) + `org.eva.capture.v1` (h1, ~32 bytes in the manifest) |
| At prove time (Level 0) | Decode the recording to regenerate witnesses; no extra storage needed |
| At prove time (Level 1/2, pre-encode hash) | Need pre-encode macroblocks: either from lossless recording or from explicit macroblock storage |
| Proof generation input | Witnesses + edit config + Nova public params → Groth16 proof |

What the camera stores at capture is lightweight: only h1 in the manifest. The witnesses — whether
regenerated from the recording or stored separately — are the prover's responsibility.

### Level 0 — software-only, no hardware changes

A trusted application (running on the camera's companion device or immediately after transfer)
reads the recording, decodes it, converts to macroblocks, computes h1 (`hash_recorder`), and
embeds `org.eva.capture.v1` in a C2PA manifest. This is entirely software: no firmware or silicon
changes are needed from the manufacturer.

Security: the gap between shutter press and the app running is unprotected. An attacker with
access to the device before the app runs could substitute a different recording. This level is
useful for validating the C2PA assertion schema and verifier tooling without any hardware changes.

### Level 1 — SDK callback inside a TEE

The camera manufacturer's SDK exposes a callback with decoded YUV frames or macroblock data
**before** they are handed to the container muxer. This callback runs inside the vendor's trusted
process, specifically a hardware TEE — for most camera and drone SoCs this is ARM TrustZone's
"secure world." The Griffin hash library is deployed as a Trusted Application (TA) using the TEE
vendor's SDK (e.g. OP-TEE for TrustZone). The camera app calls into the secure world via the
TEE API for each frame; hashing and signing happen in the secure world; the result (h1 and its
signature) is passed back.

**What the API looks like:**

```rust
sdk.on_macroblock(|frame: &MacroblockFrame| {
    // call into TEE secure world; Griffin TA hashes the macroblock
    tee_call::eva_hash_update(frame);
});
sdk.on_capture_end(|| {
    let h1 = tee_call::eva_hash_finalize();
    let sigma = tee_call::sign_with_device_key(h1);
    // write org.eva.capture.v1 into the C2PA manifest
});
```

**Do cameras expose pre-encode macroblocks today?** Partly. RAW and ProRes RAW export APIs on
several professional cameras, and Android Camera2/iOS AVCaptureSession on phones, expose YUV
frames before encoding. These are full frames, not pre-tiled; macroblock tiling would happen
inside the callback (or inside the TEE TA). DJI's camera SDK and similar proprietary SDKs expose
various pipeline stages, with specifics varying by product.

**What still needs to be integrated even with SDK access:** the SDK callback gives access to pixel
data, but Eva-specific work must be added: the Griffin TA (not SHA-256; different construction),
macroblock tiling if the API delivers full frames, writing `org.eva.capture.v1` into the manifest
at the same point the BMFF binding is written, and ensuring all of this runs inside the TEE.

**Security boundary at Level 1:** the TEE protects the hash *computation* — a compromised OS
cannot read or alter what runs in the secure world. What the TEE does not protect is the
*input* to the hash: the pixel data arrives from the encoder SDK, which is software. A
compromised camera SDK or a manipulated encoder input could pass fabricated pixels to the TEE
callback. The TEE would then faithfully hash the fake pixels. For the threat model Level 1
addresses — an editor makes edits after capture and wants to prove those edits are the only
changes — this is sufficient. Level 1 does not cover sensor-level or ISP-level fabrication of
pixel data before the SDK delivers it.

### Level 2 — dedicated hash engine on the pixel bus

A fixed-function hardware block sits on the raw YUV bus between the ISP output and the encoder
input, computing Griffin hashes over macroblock tiles before any software — including the TEE —
sees the pixels. This is the most principled design: a compromised OS, SDK, or encoder cannot
alter the hash because it is computed in dedicated silicon from the raw pixel stream.

The distinction from Level 1 is the *point in the pipeline* where hashing happens:
- Level 1 (TEE): the hash function itself is protected by the TEE, but pixels arrive from software
  (the SDK callback). Fabricated pixels fed by a compromised SDK would be hashed faithfully.
- Level 2 (silicon): pixels are hashed directly from the hardware bus before any software
  (including the TEE) has access. Even a compromised TEE TA cannot intercept or alter the data
  that was hashed.

Both levels protect the hash function from tampering; the difference is the trust boundary on the
input data.

What this requires from the manufacturer:

- A silicon tap on the ISP → encoder data bus or a dedicated DMA path to a crypto block.
- A hash block computing Griffin permutations at video-capture speed. Griffin is a simple
  arithmetisation-friendly permutation; at 4K 30fps with 16×16 macroblocks this is approximately
  500k hash invocations per second — feasible in a small dedicated block at much lower power than
  a general-purpose SHA engine running at the same rate.
- Firmware routing the tiled macroblock stream to the hash engine and delivering the output to the
  secure element for signing.
- Secure element / device key infrastructure (already present in cameras doing C2PA signing).

Level 2 requires joint specification of the data path and hash engine interface with the camera
SoC or module vendor.

### Level 3 — published standard, independently adopted

Level 2 produces one bespoke hardware integration per manufacturer. Level 3 is not a new hardware
capability: the hardware requirements are the same as Level 2. What changes is the process and
scope: the assertion schema (`org.eva.capture.v1`, `org.eva.edit_proof.v1`), the required Griffin
parameters, and the hash engine interface are published as an open specification — analogous to
how `c2pa.hash.bmff.v3` is now independently implemented by Adobe, Leica, Truepic, and others
without per-vendor customization. Any manufacturer implementing the spec produces manifests that
any Eva-aware verifier can check. This is the long-term goal; it is not a prerequisite for
Levels 0, 1, or 2.

## 7. Drones

Drones are a natural fit for this integration, in several respects better suited than consumer
phone cameras:

| Factor | Drones | Phones |
|--------|--------|--------|
| Threat model | High-stakes: drone footage used in journalism, legal proceedings, insurance claims, conflict documentation; deep-fake risk is real | Moderate to high |
| Hardware control | Manufacturer controls the full stack (sensor, ISP, encoder, flight controller SoC) — a single SDK or firmware update reaches the whole product line | Highly fragmented: Android/iOS app layers, many SoC vendors, constrained by platform OS |
| Regulatory pressure | Growing (FAA remote ID, EU drone regulation, defense sector chain-of-custody requirements) — authenticity metadata is already mandated in some contexts | Less formal regulation |
| Existing security infrastructure | Professional and military drones already carry secure elements, encrypted telemetry, and device certificates for remote ID; same infrastructure can host the Eva device key | Present only on some recent flagship phones |
| Capture context | Sensor + ISP are under operator control in a known environment; harder to intercept mid-flight without physical access | Phone can be compromised at OS level by apps |

**Storage.** Eva's macroblock witnesses are large when stored uncompressed, but are not a
bottleneck in practice. At 1080p, raw pre-quantisation YUV (full planar) is approximately
700MB/min. However, witnesses do not need to be stored uncompressed: they can be kept as the
original recording and regenerated by decoding at prove time (Level 0 / hash-over-decoded path).
In that case, witnesses cost nothing to store beyond the recording itself. If a lossless or
near-lossless recording format is used (ProRes, lossless H.264), the file runs 30–100MB/min at
1080p, comparable to high-bitrate H.264. A 64GB microSD card — standard on current commercial
drones — holds several hours of this without issue.

**Compute for hashing.** Griffin hashing is cheap and linear in macroblock count. The flight
controller's existing ARM core or a small companion-computer MCU can compute h1 in real time for
1080p footage without thermal or power impact. Nova IVC + Groth16 proof generation is heavier and
happens post-flight on a workstation or server, not on the drone.

**Signing on the drone.** Signing happens onboard — no connectivity is required for the capture
side. The flight controller or companion computer holds a device private key in its secure
element. At the end of a recording, h1 is finalised and signed with that key, and the resulting
`org.eva.capture.v1` assertion is written into the C2PA manifest alongside the recording on the
microSD card. This is the same signing architecture as remote ID transmission (which already uses
a device key on many professional drones) and any existing C2PA drone implementation. Connectivity
may be needed later for certificate management or for distributing the manifest, but the capture
and signing step is entirely offline.

**Firmware integrity.** The threat Level 1 does not address — a drone with compromised firmware
forging h1 — is the same threat that applies to any Level 1 deployment. The defences are the
same: secure boot, signed firmware updates, and (at Level 2) a hardware pixel-bus tap before
firmware can touch the data.

**Realistic Level 1 scenario for a drone manufacturer:** the drone's companion computer already
runs a signing daemon producing remote ID messages and C2PA manifests. Adding Eva support means:
link the Griffin TA, add a TEE callback that tiles the encoder's input YUV into Eva macroblock
format, feed it to the Griffin hasher, and write h1 into `org.eva.capture.v1` in the same
manifest-building pass. The manifest is written to the microSD card alongside the recording.
Onboard compute addition: negligible. Storage addition: the manifest grows by ~100 bytes.

## 8. What the camera/drone provides and what the prover provides

| Party | Provides | When |
|-------|----------|------|
| Camera / drone | Compressed recording (MP4 or lossless); `org.eva.capture.v1` (h1 + sigma, ~100 bytes in manifest); standard C2PA manifest | At capture |
| Prover (editor) | Macroblock witnesses — either regenerated by decoding the recording, or stored separately if pre-encode hash is used; edit configuration; Nova public params; Groth16 proving key | At prove time |
| Output | Edited video; `org.eva.edit_proof.v1` (Groth16 proof + h1 + h2 + gadget config); C2PA manifest linking capture and edit | After proving |

The camera's contribution is lightweight: a few hundred bytes of hash state during capture, a
~100-byte manifest entry, and tens of KB for the Griffin library. The heavy proving work (Nova IVC
+ Groth16) runs on the prover's workstation.

## 9. Does `c2pa.hash.bmff.v3` still need to be present?

Yes — for two reasons:

1. **C2PA compliance.** A conformant manifest for an ISO BMFF asset must carry a
   `c2pa.hash.bmff.v3` hard binding; without it, existing C2PA verifiers reject the manifest.
   `org.eva.capture.v1` is additive, not a replacement.
2. **Different hash domains.** The BMFF binding hashes compressed container bytes; the Eva
   binding hashes decoded macroblock pixels. They will never be numerically equal. Both are
   needed: BMFF for container tamper-detection by standard tools, Eva for pixel-level proved-edit
   chains by Eva-aware tools.

## 10. Design decisions to resolve during implementation

**Hash domain: post-ISP YUV.** The camera commits to post-ISP YUV (what Eva's `orig_*_enc`
captures today). ISP processing (demosaic, noise reduction, tone mapping) is *not* covered by the
proof; it is performed in trusted firmware. The assertion schema should carry a
`pipeline_stage: "post-isp"` field so verifiers know what the hash covers.

**Pre-encode vs post-decode hash for Levels 1/2.** If the callback fires before encoding, h1 is
over pre-encode pixels, which cannot be recovered from a lossy recording. This requires either
lossless recording or explicit macroblock storage. If the callback uses the same decode path as
Level 0 (hash over decoded-equivalent pixels), witnesses can be regenerated from the recording
but the hash is less strongly bound to the capture pipeline. This tradeoff needs to be fixed in
the spec before silicon integration.

**Griffin in software (Levels 0/1), dedicated block (Level 2).** At Level 0/1, Griffin runs as
a TEE Trusted Application. The existing SHA-256 hash engine (used for the BMFF binding) is not
reused — Griffin is a different construction. At Level 2, a dedicated Griffin block is
preferable, though a TEE-based implementation running in parallel with a SHA-256 engine for the
BMFF binding is also acceptable.

**Key reuse.** Reuse the camera's existing C2PA device certificate/key for `org.eva.capture.v1`.
This avoids a second PKI and is compatible with existing C2PA trust lists. The device's public
key becomes `vk` in the Groth16 proof; the same private key signs h1 to produce sigma.

**Proof embedding vs remote manifest.** Embed h1, h2, and the compact Groth16 proof bytes
(~200 bytes for BN254) in the manifest assertion. For very long videos, use C2PA's remote
manifest reference for a larger proof object with only h1/h2/proof-hash inline. The schema
should specify a `proof_uri` field as an alternative to inline bytes.

**Chained edits.** Each edit step produces its own `org.eva.edit_proof.v1` referencing the
previous manifest as ingredient. This matches C2PA's multi-hop ingredient model and allows
different tools to perform different edits in separate sessions without re-proving from scratch.

**IVC step boundary alignment.** Align `BLOCKS_PER_STEP` (currently 256 macroblocks) with C2PA's
optional segment hashes so a verifier can check only the segments covering a time range of
interest.

## 11. Concrete implementation path

1. **Define the assertion schema** as a CBOR/JSON spec: fields for `org.eva.capture.v1`
   (hash algorithm id, Griffin parameters, curve, h1 encoding, pipeline stage) and
   `org.eva.edit_proof.v1` (gadget id, compactified config encoding, h1, h2, proof format
   version, inline proof bytes or `proof_uri`).
2. **Build a software prototype** using `c2pa-rs` (the official Rust C2PA SDK — a natural fit
   since Eva is already Rust) wired to the existing `yuv_to_macroblocks` + `hash_recorder` +
   `edit_lossless_decider` pipeline. This produces and verifies real C2PA manifests with both
   assertion types from today's code.
3. **Write an Eva-aware C2PA verifier extension**: given a manifest pair, extract the new
   assertions, run the Groth16 verifier, and cross-check h1 across the ingredient chain (§5,
   step 3).
4. **SDK integration:** work with a camera or drone partner to identify the callback point in
   their capture SDK where pre-encode YUV is available, compile the Griffin TEE TA, and wire the
   hash + signing step into their existing manifest-writing pass. The software prototype serves as
   the integration target.
5. **Level 2 hardware specification:** once Level 1 is validated with real footage, publish a
   detailed specification for the hash engine interface — data path, tile format, Griffin
   parameter set, output protocol to the secure element — for adoption by silicon partners.

## 12. Related code and docs

| Piece | Role |
|-------|------|
| [`capture-signing-and-ingest.md`](capture-signing-and-ingest.md) | What Eva's h1/h2 bind to today; the ingest trust gap this proposal closes |
| [`c2pa_camera_architecture.md`](c2pa_camera_architecture.md) | C2PA/camera background: bindings, trust boundary, signing points |
| [`hash_recorder`](../video/examples/hash_recorder.rs) | Reference h1 computation over `orig_*_enc` |
| [`hash_orig_macroblock`](../video/src/edit_only.rs) | Per-macroblock partial h1 |
| [`video/src/decider.rs`](../video/src/decider.rs) | Groth16 decider: sigma verification (lines 399–474), verifier public inputs (lines 749–767) |
| [`docs/edit-only/README.md`](edit-only/README.md) | Lossless edit-only proof (the proof type `org.eva.edit_proof.v1` would carry) |
