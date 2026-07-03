# Capture, signing, and ingest

Eva’s zero-knowledge proofs operate on **macroblock pixel witnesses** (`orig_y_enc`,
`orig_u_enc`, `orig_v_enc`). This document describes what that assumes about real cameras,
what the proof and signature bind to, and what happens outside the circuit when you ingest
MP4 or YUV files.

Applies to the default **edit + H.264 encode** path and the **edit-only (lossless)** path on
branch `edit-only-proof`. See [`edit-only/README.md`](edit-only/README.md) for lossless-specific tooling.

## Do real cameras record `orig_*_enc`?

**No.** Consumer and most professional cameras do **not** write Eva macroblock files.
Typical capture output is:

| What cameras actually produce | Eva witness format |
|------------------------------|-------------------|
| H.264 / HEVC in MP4, MOV, etc. | Planar bytes in `orig_y_enc`, `orig_u_enc`, `orig_v_enc` |
| Optional: ProRes, RAW, HDMI/SDI uncompressed YUV | Fixed 16×16 luma + 8×8 chroma **per macroblock**, separate Y/U/V files |
| Signed at capture (if at all): container, bitstream, or file hash | Signed **h1** = Griffin hash chain over macroblock pixels |

`orig_*_enc` is a **prover witness layout** chosen to match Eva’s macroblock granularity (same
spatial tiling as H.264 macroblocks, but **pixel values in the spatial domain** — not a bitstream).
The repo’s [`hash_recorder`](../video/examples/hash_recorder.rs) example is a **reference** for
how h1 is computed over that layout. Benchmark data comes from prepared dumps (JM / Hugging Face
`data_parsed`), not from a phone camera.

Signing at capture and later proving edits does **not** plug into a normal MP4 workflow without
extra assumptions or extra proofs.

## What Eva proofs actually attest

Given macroblock witnesses `orig_*_enc` (and, on the default path, encode-side witnesses),
the IVC circuit proves a step relation and rolls Griffin hashes:

| Hash | Binds to (default / lossless) |
|------|------------------------------|
| **h1** | Original macroblock Y/U/V pixels |
| **h2** | Encoded coeffs + preds + QP + edit cfg **or** edited pixels + edit cfg (lossless) |

The Groth16 **decider** verifies a signature over **h1** (see `edit_*_decider`,
`edit_lossless_decider`, `encode_decider`). That signature is over macroblock originals, **not**
over an MP4 file.

The proof does **not** by itself prove:

- that those macroblocks are a faithful decode of a signed camera file;
- that `ffmpeg` MP4 → YUV was correct or deterministic;
- that `yuv_to_macroblocks` packing matched what a verifier would expect;
- that export back to YUV/MP4 matches the proved result.

## Where the gap appears

Typical custom-ingest workflow:

```
camera (MP4/H.264, etc.)
    │  ← if signed here, signature is NOT over orig_*_enc unless you designed for that
    ▼
ffmpeg → planar .yuv          NOT in circuit  (decode, scale, color range, …)
    ▼
yuv_to_macroblocks            NOT in circuit  (tiling, plane order, frame count)
    ▼
orig_*_enc  ──►  Eva IVC proof (edit+encode or edit-only)  ──►  h1, h2   IN circuit
    ▼
export / re-encode            NOT in circuit  (preview, H.264 mux, etc.)
```

A verifier who trusts only the zk proof learns that the circuit relation holds for the witnessed
macroblocks (e.g. edit applied, encode constraints satisfied). They do **not** automatically
learn that those macroblocks came from a particular signed recording unless you close the gap
below.

## Closing the gap

| Approach | Realistic? | Notes |
|----------|------------|-------|
| **Sign h1 at ingest** | Common for prototypes | Trusted “recorder” app: convert to macroblocks, compute h1 (`hash_recorder`), sign immediately. Camera MP4 is still outside the proof unless conversion is trusted or proved. |
| **Capture directly to macroblocks + sign h1** | Possible, not off-the-shelf | Custom firmware/app on device; signs the same layout `hash_recorder` uses. Closest to Eva’s paper model. |
| **Prove YUV → macroblocks** | Feasible future work | Deterministic packing; could be a small circuit or IVC front-end if planar YUV bytes are witnessed. |
| **Prove MP4/H.264 → YUV** | Hard | Full decode in-circuit is expensive; color range and encoder quirks matter. |
| **Signed manifest + reproducible ingest** | Weak assurance | Sign `(tool, version, width, height, fps, color_range)`; verifier re-runs ingest. Not cryptographic proof. |

For demos that start from a phone-camera MP4, ingest steps are **trusted preprocessing**. The
proof starts at `orig_*_enc`.

## What edit claims mean in practice

Example: “only brightness was changed.”

| Claim | Supported today? |
|-------|------------------|
| “Brightness `edit_circuit` was applied to these macroblock originals (h1 → h2)” | Yes (lossless path) |
| “Nothing except the chosen edit changed **in the circuit**” | Yes (for the configured gadget) |
| “Only brightness changed **relative to the signed camera file**” | Only if macroblocks ≡ signed source (see above) |

On the **default** lossy path, h2 binds predictors and coefficients after edit + encode; the
same ingest trust boundary applies to h1.

## Related code

| Piece | Role |
|-------|------|
| [`hash_recorder`](../video/examples/hash_recorder.rs) | Reference h1 over `orig_*_enc` |
| [`parse_recorder_data`](../video/src/lib.rs) | Load macroblock bytes for hashing |
| [`yuv_to_macroblocks`](../video/examples/yuv_to_macroblocks.rs) | Trusted ingest: planar YUV → macroblocks |
| [`EditEncodeCircuit`](../video/src/lib.rs) | Default IVC: edit + H.264 encode |
| [`EditOnlyCircuit`](../video/src/edit_only.rs) | Lossless IVC: edit only (branch `edit-only-proof`) |
