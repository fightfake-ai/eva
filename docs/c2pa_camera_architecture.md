# Authenticated capture and C2PA camera architecture

Background on trusted-capture hardware and C2PA (Coalition for Content Provenance and
Authenticity), written for readers of this repo. It explains what C2PA proves, where a camera's
trust boundary typically sits today, and — unlike a generic C2PA primer — ties each piece back to
what [`docs/capture-signing-and-ingest.md`](capture-signing-and-ingest.md) calls Eva's witness
format (`orig_*_enc` macroblocks, h1/h2). See
[`eva-c2pa-integration.md`](eva-c2pa-integration.md) for the forward-looking proposal on combining
the two.

## 1. What C2PA answers

C2PA (spec v2.2/2.3, 2025–2026) defines a signed, tamper-evident **manifest** attached to an
asset. A manifest is built from:

- **Assertions** — statements about the asset (capture device, edit actions, thumbnails, hashes).
- A **claim** — a digest referencing the assertions to include.
- A **claim signature** — a COSE_Sign1 signature over the claim, made with a certificate the
  claim generator (camera, editing tool) holds.

Verifying a manifest answers: *"who/what signed this, and has the asset changed since?"* It does
**not** answer *"is the pixel content itself truthful?"* — that is what Eva's zk proofs target.

## 2. Content bindings: hard vs soft

C2PA ties a manifest to specific bytes via a **content binding**:

- **Hard binding** — a cryptographic hash over asset bytes. For ISO BMFF (MP4/MOV), this is
  `c2pa.hash.bmff.v3`: a SHA-256 (or similar) hash over byte ranges of the container,
  **excluding** the box that holds the manifest itself, optionally split per-segment for large
  video files.
- **Soft binding** — a hash or fingerprint computed from *decoded content* (e.g. a perceptual
  hash or watermark), which survives re-encoding/transcoding. Used to find a manifest for a
  derived asset when the hard binding no longer matches.

**Why this matters for Eva:** the hard binding hashes **compressed container bytes**. Eva's `h1`
hashes **decoded macroblock pixels** (`orig_y_enc` / `orig_u_enc` / `orig_v_enc` — see
[`capture-signing-and-ingest.md`](capture-signing-and-ingest.md)). These are hashes over
**different representations of the same content** and will never be numerically equal. Any
integration needs an explicit assertion type for the macroblock-domain hash rather than trying to
reuse the BMFF hard binding as-is.

## 3. Where is the trust boundary in a camera today?

| Component | Typical trust level | Notes |
|-----------|---------------------|-------|
| Sensor | Partial | No cryptographic attestation that the sensor feed is genuine; a compromised sensor interface is the classic unsolved "camera of the future" attack C2PA capture assertions can't fully rule out. |
| ISP (image signal processor) | Usually trusted operationally | Runs signed/isolated firmware on many devices, but that is a vendor security property, not a cryptographic proof to an external verifier. |
| Encoder (H.264/HEVC) | Usually trusted operationally | Same caveat as ISP; often the same SoC block. |
| CPU / OS / apps | Untrusted | Assumed compromisable; C2PA capture flows try to keep signing off the general-purpose CPU. |
| Hash/crypto engine | Ideally trusted | Dedicated silicon block; smaller attack surface than general CPU. |
| Secure element / TPM | Root of trust | Holds the device signing key; the key should never leave this boundary. |

"Trusted ISP" means isolated firmware that the OS/apps cannot modify — an operational security
property, not a cryptographic proof. C2PA capture claims currently rely on this operational trust
for the *sensor → hash* leg; there is no widely deployed cryptographic attestation that the pixels
handed to the hash engine came unmodified from the sensor.

## 4. Where signing can happen, and why it matters for Eva

| Point in pipeline | What gets hashed/signed | Detects tampering after… | Compatible with Eva h1 as-is? |
|--------------------|--------------------------|---------------------------|-------------------------------|
| A. CPU-based, post-decode | Decoded YUV frames, hashed on the general CPU | Sensor → ISP → encoder → decode | Closest match to Eva's macroblock domain, but CPU is untrusted — a compromised CPU can hash *and* alter pixels before Eva ever sees them. |
| B. Dedicated hash engine on the raw/decoded video bus | Same domain as A, but hashed by fixed-function silicon before the CPU can touch it | Sensor → ISP → encoder | Best fit for Eva if the engine hashes in Eva's macroblock layout, or if Eva's own hash (Griffin) can be computed by that engine. |
| C. Hash after encoding (most common in shipping products, e.g. Leica M11-P for still images) | Encoded container bytes | Only after final encode/mux | Wrong domain for Eva's h1 (compressed vs spatial pixels); would need a decode step, re-introducing an unproved gap. |

Only **hashes** are signed, not the video itself — a rolling/incremental hash over the chosen
domain (frames, macroblocks, or container segments) keeps memory bounded even for a large clip;
the final digest is what the secure element signs.

**Leica M11-P** is the one shipping consumer example of in-camera C2PA signing
(<https://leica-camera.com/en-US/photography/content-credentials>,
<https://c2paviewer.com/articles/leica-m11-p-c2pa>) — but it signs **still photos**, not video, so
it validates the "trusted signing at capture" pattern without validating any specific video
hashing granularity.

## 5. What C2PA does and does not prove

C2PA proves:

- Who (or what device/tool) signed the manifest.
- That the asset has not changed since that signature, relative to the declared content binding.

C2PA does **not** prove:

- That the pixel content is a truthful record of the scene (no relation to scene realism).
- That nothing was altered *before* the hash was taken (ISP/encoder correctness is assumed, not
  proved).
- Anything about what changed between two signed versions of related content, beyond whatever an
  **actions** assertion *claims* — which is asserted by the editing tool, not cryptographically
  enforced against the pixels.

That last point is exactly the gap Eva is built to close: instead of an editing tool merely
*asserting* "I only adjusted brightness," Eva can produce a succinct proof that the edited
macroblocks are the result of applying a specific, bounded edit gadget to the originals bound by
h1. See [`eva-c2pa-integration.md`](eva-c2pa-integration.md).
