# Masks

Pipeline stage 6 carrier ([pipeline](pipeline.md)): every way a local adjustment
selects pixels. Owning code: `focale-core/src/masks.rs`,
`focale-core/src/pipeline/v1/masks.rs` (rasterization), `focale-segment` (AI
segmentation). Governing invariants: `[HARD-DET]`, `[HARD-LOCAL]`,
`[HARD-LICENSE]` ([invariants](../invariants.md)).

## Parity definition (normative — v1 ships all of this, nothing less)

**Geometric:** brush (size/feather/flow, eraser), linear gradient, radial
gradient. **Range:** luminance range, colour range (sampled, with
tolerance/falloff). **AI-segmented (local ONNX models, `[HARD-LOCAL]`):** subject,
sky, background, objects (click/brush-to-select), people — with per-person
components: face skin, body skin, hair, eyebrows, eyes (sclera + iris/pupil),
lips, teeth, clothing. **Operations:** any mask combinable via add / subtract /
intersect / invert; per-mask feather and density (max-opacity) controls; masks
nest into named groups.

Status: `v1 (shipped)`, with the two model-capability limits noted below.

## Implementation

- Geometric and range masks are stored **parametrically** in the sidecar and
  rasterized on the CPU with fixed iteration order; rasterization is part of the
  versioned pipeline (`[HARD-DET]`).
- AI masks are resolved at creation time into 8-bit coverage bitmaps at 1/2 the
  segmentation input resolution (the preview base in the app — quality/size
  balance), deflate-compressed in the sidecar *(chosen over vectorization —
  exact, simple, deterministic; vectorization would lose the model's soft
  edges)*. Export upsamples bilinearly — deterministic, versioned. Because
  resolution happens at creation time, exports never re-run a model — this is
  what keeps AI masks compatible with `[HARD-DET]`, and it also means future
  model upgrades never break old sidecars.
- Mask algebra: add / subtract / intersect / invert over f32 coverage in [0,1];
  per-mask feather (Gaussian, fixed kernel) and density (max-opacity scale);
  masks nest into named groups.
- Wire format for all of the above: [sidecar](sidecar.md) §5.8.

## Segmentation stack

- Runtime: `ort` (MIT) with ONNX models — the runtime decision and its rationale
  are owned by [rnd/inference.md](../rnd/inference.md) (mask-time inference is
  off the export path, so bit-determinism is not required of it).
- Models: MobileSAM (Apache-2.0) for subject/object click-to-select, a
  BiSeNet-family face parser (MIT) for people components, and a U²-Net-family
  sky/background model (Apache-2.0). Models run on 8 GB GPUs or CPU (slow), and
  are loaded from the user data directory; the app ships a downloader script and
  shows a "model not installed" affordance (the app itself makes no network
  calls, `[HARD-LOCAL]`). Distribution mechanics, per-model licenses, and the
  model manifest are owned by [rnd/ml-models.md](../rnd/ml-models.md).
- Alternatives considered: SAM 2.1 (Apache-2.0 weights, better masks, but a far
  heavier image encoder and a video-centric API), EfficientSAM (similar trade),
  BiRefNet (MIT, clearly superior background/subject matting but ~973 MB fp32
  ONNX — the leading `v2 (committed)` candidate once lite variants are
  evaluated). Since masks resolve into the sidecar, upgrading models later is
  cheap; shipping v1 on the smaller proven stack is correct.
- Known v1 model-capability limits (not implementation limits): single person
  only — the face parser runs on the full frame and always reports person index
  0 (`v1 (gap, issue #8)`); sclera and iris resolve to the same eye region
  because the CelebAMask-HQ 19-class label set has one eye class per side
  (`v1 (gap, issue #9)`).
