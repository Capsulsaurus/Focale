# Focale Architecture

This document records every design decision delegated to the implementing agent by
`docs/prd.md` (*"agent's choice"*), plus the load-bearing structure that follows from
the PRD's HARD requirements. Keep it current: any change to a decision below must be
reflected here in the same PR.

## 1. Workspace layout

| Crate | Role |
| --- | --- |
| `focale-core` | Everything on the deterministic path: decode wrapper, pipeline stages, colour math, mask rasterization, retouch, geometry, export encoding. CPU-only, no GUI deps. |
| `focale-sidecar` | The `.fcl` sidecar: schema types, RFC 8949 §4.2 deterministic CBOR. |
| `focale-segment` | ONNX segmentation (ort). Used only at mask-creation time; never on the export path. |
| `focale-cli` | Headless export binary. The reference deterministic path; CI runs it on x86_64 + aarch64 and diffs bytes. |
| `focale-app` | Desktop GUI (winit + wgpu + egui). Depends on core/sidecar/segment. |

**Rationale:** the export path must be testable headless on CI across architectures,
so nothing in `focale-core` may depend on GUI or GPU crates. `focale-segment` is
isolated because model inference is explicitly *not* part of the deterministic path
(masks are resolved into the sidecar at creation time, PRD §4).

## 2. Sidecar encoding *(agent's choice: implementation strategy)*

- Serde structs bridge through `ciborium::Value`, but bytes are produced by our own
  canonical writer (`focale-sidecar::cde`): shortest-form integers/lengths, definite
  lengths only, map keys sorted bytewise by encoded key, floats in the shortest of
  f16/f32/f64 that round-trips, NaN canonicalized to `f97e00`.
- **Rationale:** byte-stability forever is a HARD guarantee; we cannot pin it to a
  third-party encoder's internals. Reading accepts any well-formed CBOR.
- Struct field names are the map keys (text). Renaming a field is a schema change and
  requires a schema version bump; the old name must remain readable forever.
- Resolved AI masks are stored as deflate-compressed 8-bit bitmaps (see §6).

## 3. Raw decode (PRD-fixed: rawshift; details are agent's choice)

- `rawshift-image` with `arw` + `dng` features (the crate's two "stabilizing"
  formats). Supported today: Sony lossless-compressed ARW (compression type 7 —
  A7 IV / A7R V / A1 generation bodies) and DNG. Uncompressed and lossy ARW decode
  will arrive upstream; we surface a clear per-file error until then.
- Path: `decode_raw()` (u16 CFA) → black-level subtract → demosaic → normalize
  `u16 / white_level` to **linear camera RGB f32**.
- **Demosaic is pinned per pipeline version** to `Bayer(Amaze)` — never `Auto`,
  which is content-adaptive and would break the permanent-versioning rule.
  rawshift's row-parallel demosaic writes disjoint rows from immutable input, so it
  is deterministic under any thread count.
- Camera colour matrix (XYZ→camera, DNG convention) comes from rawshift's bundled
  per-model database with dual-illuminant CCT interpolation; DNG files use their
  embedded `ColorMatrix1/2`. `gamut` is not needed at the decode stage (it is a
  codec library); it was evaluated and its current versions lack what export needs
  (see §7).

## 4. Optical corrections

- v1 source is exclusively embedded metadata (PRD HARD). Implemented behind a
  `CorrectionSource` trait so a v2 external profile database can slot in.
- Reality of the v1 decode stack: rawshift 0.1.1 parses **no** optics metadata from
  ARW (Sony stores it in undecoded MakerNote tags) and only DNG `GainMap` opcodes on
  its internal DNG path. Consequently the stage emits the PRD-mandated visible
  warning ("no optics metadata; stage skipped") for affected files. The stage, its
  UI panel, its warning plumbing, and the seam are fully built so corrections light
  up as upstream metadata parsing lands.

## 5. Processing pipeline

- Fixed stage order per PRD §3, working space **linear Rec.2020, f32, unbounded**.
- White balance: as-shot / kelvin+tint / custom multipliers, applied in camera RGB
  before the camera→working transform (Bradford CAT for illuminant adaptation).
- Every stage is a pure function `(&StageInput, &Params) -> StageOutput` keyed by
  pipeline version. Version 1 algorithms live in `focale_core::pipeline::v1` and are
  frozen at release; changing output requires adding `v2` modules while `v1` stays.
- **Determinism rules on the export path:** CPU only; `rayon` permitted solely for
  disjoint-row/tile maps (no reductions across threads); histograms and any
  whole-image statistics are computed sequentially in fixed order; no `HashMap`
  iteration touches pixels; no `fast-math`, no FMA-dependent algorithms (only
  individually rounded f32 ops).
- Geometry (crop/rotate/perspective) is applied at its PRD position (stage 9) in
  both preview and export. *(agent's choice §3.9)*: preview does **not** composite
  geometry earlier; the viewport simply draws the geometry-stage output, which keeps
  export math and preview framing identical by construction.

## 6. Masks

- Geometric masks (brush strokes, linear/radial gradients) and range masks
  (luminance, colour) are stored **parametrically** in the sidecar and rasterized on
  the CPU with fixed iteration order; rasterization is part of the versioned
  pipeline.
- AI masks are resolved at creation time into 8-bit coverage bitmaps at 1/2 the
  segmentation input resolution (the preview base in the app — quality/size
  balance), deflate-compressed in the sidecar
  *(agent's choice: compressed bitmap over vectorization — exact, simple, and
  deterministic; vectorization would lose the model's soft edges)*. Export upsamples
  bilinearly — deterministic, versioned.
- Mask algebra: add / subtract / intersect / invert over f32 coverage in [0,1];
  per-mask feather (Gaussian, fixed kernel) and density (max-opacity scale); masks
  nest into named groups.
- Segmentation stack *(agent's choice)*: `ort` (MIT) with ONNX models —
  MobileSAM (Apache-2.0) for subject/object click-to-select, a BiSeNet-family face
  parser (MIT) for people components, and a U²-Net-family sky/background model
  (Apache-2.0). Models are redistributable, run on 8 GB GPUs or CPU (slow) and are
  loaded from the user data directory; the app ships a downloader script and shows a
  "model not installed" affordance (the app itself makes no network calls).

## 7. Colour management & export *(agent's choice: codecs and operators)*

- All primaries/transfer math is implemented in `focale-core::color` from the
  published primaries (sRGB/Rec.709, Display P3, Adobe RGB, Rec.2020) with Bradford
  chromatic adaptation; unit-tested against known reference values.
- **Preview:** the viewport is our wgpu render pass; a fragment shader converts
  working-space linear Rec.2020 → the surface's colour space. v1 queries the
  compositor where possible and otherwise uses the PRD fallback: user-set display
  profile, sRGB default. The active rendering gamut (sRGB / Display P3 / Adobe RGB)
  is a user setting shown in the status bar. A proofing-transform seam sits between
  the working image and the display transform (PRD §5 deferred item).
- **Export codecs** (PRD delegates the choice; `gamut` 0.3 was evaluated — pure Rust
  and deterministic, but currently 8-bit SDR TIFF/AVIF/WebP only, no JXL/PNG/JPEG/
  16-bit/HDR/ICC, so it cannot satisfy §8; revisit as it matures):
  - TIFF 16-bit: `tiff` (MIT) — the hand-off format, with embedded ICC.
  - PNG: `png` (MIT/Apache) — 16-bit, cICP chunk for HDR (PQ) + ICC for SDR.
  - JPEG: `jpeg-encoder` (MIT/Apache) — 8-bit + ICC.
  - JPEG XL: `jpegxl-rs` (GPL-3.0-or-later bindings over BSD-3 libjxl; GPLv3 §13
    makes GPL-3 works combinable with AGPL-3.0) — 16-bit, lossless option, HDR
    (PQ/HLG).
  - AVIF: `rav1e` (BSD-2) + `avif-serialize` (BSD-3) — 10-bit, CICP-signaled
    PQ/HLG and wide gamut. Encoders run single-threaded with pinned settings so
    output bytes are reproducible.
  - All licenses verified AGPL-3.0-compatible.
- **HDR→SDR tone mapping** *(pipeline-versioned)*: extended Reinhard
  (white-point-preserving) on max-RGB in linear light. **Gamut mapping**
  *(pipeline-versioned)*: hue-preserving chroma compression in Oklab — binary
  search on the (a,b) scale at constant L, fixed 20 iterations, no trig on the
  mapping path. Gain-map export is deferred (seam kept in the export recipe schema —
  a recipe carries an optional `gain_map` block, rejected at execution in v1).

## 8. Preview architecture *(agent's choice)*

- **One implementation of the pipeline math.** Decode happens once per image; the
  result is immediately box-downscaled to a preview base (long edge ≤ 2560 px) and
  the full-resolution buffer dropped. Every slider change re-runs the CPU pipeline
  on that base — at this size a full re-run stays inside the latency budget, so v1
  ships without per-stage caching (the seam for it is the preview scheduler). The
  GPU does exactly one thing: the colour-managed blit (working→display) plus
  zoom/pan sampling — its WGSL mirrors the CPU tone-map and gamut-map operators
  and receives every colour matrix from `focale_core::color` constants.
- **Rationale:** the PRD requires the GPU preview to be perceptually faithful to
  the CPU path forever, across every pipeline version. Duplicating eleven stages in
  WGSL doubles every algorithm and every version freeze; at preview resolution
  (≈4 MP) the cached CPU pipeline meets the <100 ms slider-to-screen budget on the
  16-core reference machine with headroom. If profiling ever falsifies that, the
  seam is the preview scheduler, not the stage code.
- Preview quality: fit-to-window renders on a mip of the demosaiced image; 1:1 zoom
  renders the visible tile at full resolution.

## 9. Compute scheduling & AI-suggestion stub

- A priority job scheduler owns all background work per opened file:
  interactive preview > thumbnail/filmstrip > export queue > **idle** work.
- The v1 suggestion engine is a stub implementing the full v2 contract: it runs when
  the file's queue goes idle (or immediately on demand), produces per-slider
  `Suggestion { stage, param, value }` proposals, and the UI renders
  accept / tweak / ignore affordances. The stub returns "no suggestions"; the
  scheduling, plumbing, and UI ship in v1.

## 10. Session, batch, status bar

Straight from PRD §7 (no delegated choices): one directory per session, sidecar-only
index (rating/flag/label + capture-time cache + thumbnail hash live in the sidecar's
live-index block), copy/paste settings to multi-selection, multi-select edit
broadcast, background export queue, persistent keyed status bar (active gamut,
pipeline version, cursor colour, zoom, warnings).

## 11. PRD §10 deliverables map

| Deliverable | Where |
| --- | --- |
| Architecture decisions with rationale | this document |
| Sidecar schema doc + deterministic-encoding golden tests | `docs/sidecar-schema.md`; `focale-sidecar/tests/golden.rs` (committed `canonical.fcl`, double-serialize and map-order-permutation byte equality) |
| Determinism CI across x86_64 + aarch64 | `.github/workflows/determinism.yml` (renders `synthetic.dng` + `determinism.fcl` in every format on both arches and diffs hashes); in-process double-encode guard in `focale-cli/tests/determinism.rs` |
| Pipeline-version regression suite | `focale-core/tests/pipeline.rs` (`rich_edit_matches_frozen_golden` — frozen hash; already caught one real glibc transcendental divergence, fixed by routing through `focale_core::math`/libm) |
| Colour tests (working-space → display/export values per gamut/format) | `focale-core/src/color/*` (48 reference-value tests: matrices re-derived in f64, transfer round-trips, PQ/HLG anchors, Oklab, per-gamut mapping) + `focale-export/tests/export.rs` numeric probes (sRGB 16-bit value, PQ 203-nit value, cICP payloads, per-gamut encodes) |
| Mask parity checklist (§4) as integration tests | `focale-core/tests/masks.rs` (28 tests: every shape, every op, feather/density, groups, determinism) + `focale-segment` unit/integration tests (subject/sky/background/object/person + parts) |
| 20-in-30 workflow possible end-to-end | tooling shipped: keyboard culling, multi-select edit broadcast, copy/paste settings, background export queue; final timing validation requires a real shoot on a desktop session |

## 12. Determinism CI

- `focale-cli render <raw> <sidecar>` is the canonical export entry point.
- CI job matrix: `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64) render the
  committed fixture set and compare SHA-256 of output bytes; any divergence fails.
- Golden-file suites: (a) sidecar bytes for a canonical edit state, (b) frozen
  sidecars + frozen output hashes per pipeline version (regression), (c) colour
  transform vectors per gamut/format.
