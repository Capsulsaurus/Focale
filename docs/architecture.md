# Focale Architecture

The canonical specification of Focale: product requirements, every design decision
with its rationale, and the load-bearing structure that follows. Keep it current:
any change to a decision below must be reflected here in the same PR. Requirements
marked **HARD** are non-negotiable; each carries a stable ID (e.g. `[HARD-DET]`)
that code comments cite.

Remaining v1 delivery gaps are tracked as sub-issues of
[#1 (V1 Roadmap)](https://github.com/Capsulsaurus/Focale/issues/1); this document
notes the relevant issue number wherever a described seam is not yet implemented.

## 1. Product definition & non-negotiables

A desktop raw photo developer for experienced photographers who want deliverable
results fast. One fixed, correctness-ordered processing pipeline; creative
flexibility expressed through masks and (v2) AI-suggested parameter values — never
through panel sprawl or reorderable node graphs.

**Success metric (acceptance bar):** an experienced photographer takes 20 culled
raw images and finishes editing them in 30 minutes, at output quality matching what
they would produce in the raw editor of their choice. (Validation run tracked on
issue #1.)

**Explicit non-goals:** cataloging/DAM (users pair with external tools), heavy
retouching (users export 16-bit TIFF to Photoshop-class tools), cloud anything,
telemetry, accounts.

### The five HARD principles

1. **`[HARD-DET]` Determinism.** Identical (raw file + sidecar + pipeline version)
   inputs produce **bit-identical export output on any machine**. The export path
   is CPU-only, uses no non-deterministic parallel reductions, no `fast-math`,
   fixed iteration orders, and pinned algorithm versions. The GPU is used **only**
   for interactive preview and must be perceptually faithful to the CPU path, but
   bit-identity is not required of the preview.
2. **`[HARD-VER]` Permanent pipeline versioning.** Every sidecar records the
   pipeline version that created it. Newer software must recreate the identical
   export from older sidecars forever. Changing any algorithm's output requires
   introducing a new pipeline version while retaining the old implementation. No
   exceptions, no deprecation. The same permanent-compatibility rule applies to
   the sidecar schema.
3. **`[HARD-LOCAL]` Local-only.** All computation, including all future AI, runs
   on the user's machine. No network calls in the application.
4. **`[HARD-LICENSE]` License.** AGPL-3.0. External contributions require a CLA
   assigning rights to the project author. All dependencies must be
   AGPL-compatible; verify before adding any crate or model weights.
5. **`[HARD-RUST]` Rust core.** All processing logic is Rust. Existing crates for
   raw decode, codecs, and math are preferred over reimplementation; do not reject
   a crate for immaturity alone.

### Deferred to v2 (do not build, do not block)

- AI-suggested slider values (neural-guided optimization toward a
  professional-corpus target; training details TBD). The v1 stub ships the full
  scheduling and UI contract (§10).
- Lens measurement kit + open correction-profile database (separate project;
  integration seam required per §5).
- Neural denoise and sharpening (new pipeline-versioned stages).
- Soft-proofing / print intent (seam noted in §8).
- Content-aware / generative retouch.

## 2. Workspace layout

| Crate | Role |
| --- | --- |
| `focale-core` | Everything on the deterministic path: decode wrapper, pipeline stages, colour math, mask rasterization, retouch, geometry. CPU-only, no GUI deps, no build script. |
| `focale-sidecar` | The `.fcl` sidecar: schema types, deterministic CBOR writer. No build script; writers pass provenance strings in. |
| `focale-export` | Output transform and export encoders. |
| `focale-segment` | ONNX segmentation (ort). Used only at mask-creation time; never on the export path. |
| `focale-buildinfo` | Build provenance strings (release version + git short hash, platform name) for the writing binaries; keeps the deterministic-path crates free of build scripts. |
| `focale-cli` | Headless export binary. The reference deterministic path; CI runs it on x86_64 + aarch64 and diffs bytes. |
| `focale-app` | Desktop GUI (winit + wgpu + egui). Depends on core/sidecar/segment/export/buildinfo. |

**Rationale:** the export path must be testable headless on CI across
architectures, so nothing in `focale-core` may depend on GUI or GPU crates.
`focale-segment` is isolated because model inference is explicitly *not* part of
the deterministic path (masks are resolved into the sidecar at creation time, §6).

## 3. Processing pipeline (normative)

**HARD (`[HARD-VER]`) — the stage order is fixed** and identical for preview and
export. Users cannot reorder stages; they can only enable/disable and parameterize
them. Order:

1. **Raw decode** — demosaic to linear camera RGB, f32 (§4).
2. **Optical corrections** — vignetting, chromatic aberration, distortion, from
   embedded raw metadata only in v1 (§5).
3. **White balance** and camera-to-working-space transform.
4. **Global tone** — exposure, contrast, highlights/shadows/whites/blacks, tone
   curve (parametric + point curve).
5. **Global colour** — HSL per-band, colour grading (shadows/midtones/highlights
   wheels), vibrance/saturation.
6. **Local adjustments** — any subset of stages 4–5 parameters applied through
   masks (§6).
7. **Detail** — capture sharpening (unsharp/deconvolution) and conventional noise
   reduction (luma/chroma). Non-neural in v1; v2 neural replacements arrive as new
   pipeline-versioned stages.
8. **Retouch** — heal and dust-spot removal (clone/heal brush). Content-aware
   inpainting is out of scope for v1.
9. **Geometry** — crop, rotate, perspective. Applied at this position in both
   preview and export: the viewport simply draws the geometry-stage output, which
   keeps export math and preview framing identical by construction (no earlier
   compositing).
10. **Finishing** — post-crop vignette, grain.
11. **Output transform** — working space → target colour space + tone mapping
    (§8), executed by `focale-export`.

**Working space (HARD, `[HARD-DET]`):** linear Rec.2020 primaries, f32, unbounded —
values may exceed [0,1] (and camera colours outside Rec.2020 survive as negative
components) until the output transform. Rationale for Rec.2020 over the
alternatives: it is the working space of darktable's scene-referred pipeline and
matches the linear-primaries philosophy of Lightroom's internals; ACEScg (AP1) is
only marginally wider and buys nothing for a stills developer that does not
interchange with VFX pipelines; ProPhoto/Melissa has imaginary primaries (physical
nonsense values that make per-channel operations behave unintuitively) and a D50
white point forcing an extra adaptation; Oklab-style spaces are non-linear and
non-radiometric — blurs, resampling, and blending are physically wrong in them, so
Oklab is used at the operator level instead (§8). Decisive extra: every HDR
container (PQ/HLG CICP) is built around Rec.2020, so the HDR export path has an
identity primary transform.

**Determinism rules on the export path (`[HARD-DET]`):** CPU only; `rayon`
permitted solely for disjoint-row/tile maps (no reductions across threads);
histograms and any whole-image statistics are computed sequentially in fixed
order; no `HashMap` iteration touches pixels; no `fast-math`, no FMA-dependent
algorithms (only individually rounded f32 ops); all transcendentals route through
`focale_core::math` (pure-Rust libm — std float functions differ across glibc
versions).

**Versioning mechanics (`[HARD-VER]`):** every stage is a pure pipe-filter
function `(image, &Params)` keyed by pipeline version. Version 1 algorithms live
in `focale_core::pipeline::v1` and are frozen at release; changing output requires
adding `v2` modules while `v1` stays. A v2 reuses unchanged v1 stage functions
directly, so old versions remain re-runnable forever even when the new version's
defaults differ. The GUI and CLI both render (preview, edit, export) with the
sidecar's **stored** pipeline version; only an explicit user "upgrade" action
re-stamps it. See docs/sidecar-schema.md §3 for the full mechanism.

## 4. Raw decode & demosaic

- `rawshift-image` with `arw` + `dng` features (the crate's two "stabilizing"
  formats). Supported today: Sony lossless-compressed ARW (compression type 7 —
  A7 IV / A7R V / A1 generation bodies) and DNG. Uncompressed and lossy ARW decode
  will arrive upstream; we surface a clear per-file error until then (issue #12).
  Initial camera support target: Sony ARW first-class; other makes as the decode
  crate allows.
- Path: `decode_raw()` (u16 CFA) → black-level subtract → demosaic → normalize
  `u16 / white_level` to **linear camera RGB f32**.
- **Demosaic is pinned per pipeline version** to `Bayer(Amaze)` — never `Auto`,
  which is content-adaptive and would break `[HARD-VER]`. rawshift offers AMaZE,
  RCD, and LMMSE for Bayer (plus Markesteijn for X-Trans); AMaZE is RawTherapee's
  Bayer default and the reference for low-ISO detail recovery, while RCD trades a
  little fine detail for fewer overshoot artifacts and speed — but speed is
  irrelevant on the export path ("correct beats fast") and preview runs on the
  downscaled base, so AMaZE's quality edge wins. v2 note: LMMSE is superior on
  very-high-ISO frames; exposing the demosaic algorithm as a *recorded sidecar
  parameter* (never content-adaptive) would stay fully deterministic. rawshift's
  row-parallel demosaic writes disjoint rows from immutable input, so it is
  deterministic under any thread count.
- Camera colour matrix (XYZ→camera, DNG convention) comes from rawshift's bundled
  per-model database with dual-illuminant CCT interpolation; DNG files use their
  embedded `ColorMatrix1/2`. The `gamut` crate is not used at the decode stage (it
  is a codec library); it was evaluated and its current versions lack what export
  needs (§9).

## 5. Optical corrections

- v1 source is exclusively embedded metadata (**HARD**). If metadata is absent and
  nothing can be inferred, the stage emits a visible warning in the UI and is
  skipped — never guess, never fail.
- Reality of the v1 decode stack: rawshift 0.1.1 parses **no** optics metadata
  from ARW (Sony stores it in undecoded MakerNote tags) and only DNG `GainMap`
  opcodes on its internal DNG path. Consequently the stage emits the mandated
  visible warning ("no optics metadata; stage skipped") for affected files.
- The current seam is the `OpticsMetadata` presence struct
  (`focale-core/src/decode`) plus the `OpticsParams` stage toggles (inert until
  metadata exists) and the warning plumbing. The planned `CorrectionSource` trait
  — the v2 seam for an external profile database — and the correction math itself
  are tracked in issue #7; nothing else in this stage exists yet.

## 6. Masks

### Parity definition (normative — v1 ships all of this, nothing less)

**Geometric:** brush (size/feather/flow, eraser), linear gradient, radial
gradient. **Range:** luminance range, colour range (sampled, with
tolerance/falloff). **AI-segmented (local ONNX models, `[HARD-LOCAL]`):** subject,
sky, background, objects (click/brush-to-select), people — with per-person
components: face skin, body skin, hair, eyebrows, eyes (sclera + iris/pupil),
lips, teeth, clothing. **Operations:** any mask combinable via add / subtract /
intersect / invert; per-mask feather and density (max-opacity) controls; masks
nest into named groups.

### Implementation

- Geometric and range masks are stored **parametrically** in the sidecar and
  rasterized on the CPU with fixed iteration order; rasterization is part of the
  versioned pipeline (`[HARD-DET]`).
- AI masks are resolved at creation time into 8-bit coverage bitmaps at 1/2 the
  segmentation input resolution (the preview base in the app — quality/size
  balance), deflate-compressed in the sidecar *(chosen over vectorization — exact,
  simple, deterministic; vectorization would lose the model's soft edges)*. Export
  upsamples bilinearly — deterministic, versioned. Because resolution happens at
  creation time, exports never re-run a model — this is what keeps AI masks
  compatible with `[HARD-DET]`, and it also means future model upgrades never
  break old sidecars.
- Mask algebra: add / subtract / intersect / invert over f32 coverage in [0,1];
  per-mask feather (Gaussian, fixed kernel) and density (max-opacity scale); masks
  nest into named groups.
- Segmentation stack: `ort` (MIT) with ONNX models — MobileSAM (Apache-2.0) for
  subject/object click-to-select, a BiSeNet-family face parser (MIT) for people
  components, and a U²-Net-family sky/background model (Apache-2.0). Models are
  redistributable, run on 8 GB GPUs or CPU (slow), and are loaded from the user
  data directory; the app ships a downloader script and shows a "model not
  installed" affordance (the app itself makes no network calls, `[HARD-LOCAL]`).
  Alternatives considered: SAM 2.1 (Apache-2.0 weights, better masks, but a far
  heavier image encoder and a video-centric API), EfficientSAM (similar trade),
  BiRefNet (MIT, clearly superior background/subject matting but ~973 MB fp32
  ONNX — the leading v2 candidate once lite variants are evaluated). Since masks
  resolve into the sidecar, upgrading models later is cheap; shipping v1 on the
  smaller proven stack is correct.
- Known v1 model-capability limits (not implementation limits): single person
  only — the face parser runs on the full frame and always reports person index 0
  (issue #8); sclera and iris resolve to the same eye region because the
  CelebAMask-HQ 19-class label set has one eye class per side (issue #9).

## 7. Sidecar format

- **HARD (`[HARD-VER]`):** one sidecar per image, CBOR with deterministic
  encoding — identical documents always serialize to identical bytes. File name:
  `<image-filename>.<ext>.fcl`. Stored alongside the image; the raw file is never
  modified. Contents: schema version, pipeline version, full parameter set for
  every stage, resolved masks, retouch strokes, geometry, export recipes, a
  live-index metadata block sufficient for the directory view to build its index
  by scanning sidecars alone (§11), and debug provenance. The schema is
  forward-versioned with the same permanent-compatibility rule as the pipeline
  and published in-repo: **docs/sidecar-schema.md** is the normative format
  specification.
- Serde structs bridge through `ciborium::Value`, but bytes are produced by our
  own canonical writer (`focale-sidecar::cde`): shortest-form integers/lengths,
  definite lengths only, map keys sorted bytewise by encoded key, floats in the
  shortest of f16/f32/f64 that round-trips, NaN canonicalized to `f97e00`. The
  shortest-form/definite-length/sorted-keys rules are RFC 8949 §4.2.1 (Core
  Deterministic Encoding Requirements); the float-width and NaN canonicalization
  follow RFC 8949 preferred serialization plus the IETF CBOR CDE draft
  (draft-ietf-cbor-cde, expired at draft-13 without becoming an RFC — its
  unpublished status, and the absence of any Rust encoder guaranteeing these
  bytes, is exactly why byte production is not pinned to a third-party encoder's
  internals). Reading accepts any well-formed CBOR.
- Renaming a field is a schema change and requires a schema version bump; the old
  name must remain readable forever. Any intentional change to canonical bytes
  re-blesses the golden fixture in the same change.

## 8. Colour management

- All primaries/transfer math is implemented in `focale-core::color` from the
  published primaries (sRGB/Rec.709, Display P3, Adobe RGB, Rec.2020) with
  Bradford chromatic adaptation; unit-tested against known reference values.
- **Preview (HARD):** the app renders colour-managed; the image viewport is our
  own wgpu render pass whose fragment shader converts working-space linear
  Rec.2020 → the display space. Assume professional users on ~Display P3 D65
  hardware, but never hard-code the assumption. **Current reality:** v1 assumes
  an sRGB surface (the shader sRGB-encodes when the swapchain format is not
  already sRGB); querying the compositor (Wayland `wp_color_management_v1`,
  macOS tagged `CAMetalLayer`) and the user-set display profile are
  **unimplemented** — tracked in issues #6 and #10. The seam is the viewport
  uniform block, which already receives every colour matrix from
  `focale_core::color` constants.
- **HARD:** the GUI has a user-selectable **active rendering gamut** (sRGB,
  Display P3, Adobe RGB), always visible as a status-bar key (§11).
- **HDR→SDR tone mapping** *(pipeline-versioned)*: extended Reinhard
  (white-point-preserving) on max-RGB in linear light. This operator is a
  residual safety net that runs *after* the user has manually set exposure and
  tone — the artistic decision is the user's; the operator only disposes of
  remaining energy above 1.0 gracefully. For that role max-RGB extended Reinhard
  is the right choice: one scale factor per pixel preserves channel ratios (hue)
  exactly with zero trig; output is bounded in **every** channel, so the gamut
  mapper only ever handles primaries mismatch, never tone overflow; it is a few
  flops with no transcendentals, trivially deterministic, and exactly mirrored in
  WGSL. Trade-offs, stated honestly: the curve compresses everywhere, not just
  highlights (mid-grey 0.18 → ≈0.154 at white=4, ≈14% darkening — visible in
  preview, so users compensate with exposure, identically in preview and export),
  and saturated colours render darker than a luminance-driven operator would.
  Alternatives: BT.2446 Method C rated best in a 2025 subjective study (MDPI
  Electronics 14(12):2428) — but for *unsupervised HDR-video→SDR broadcast
  conversion of display-referred masters*, a different job; it is luminance-driven
  (individual channels can exceed the mapped peak) and needs a crosstalk matrix +
  Yxy round-trip. It remains the candidate if unsupervised batch conversion ever
  becomes a priority (new pipeline version). ACES RRT, AgX, and Hable were
  rejected for imposing a *look* (hue skews / filmic contrast) — wrong for a
  neutral safety net.
- **Gamut mapping** *(pipeline-versioned)*: hue-preserving chroma compression in
  Oklab — binary search on the (a,b) scale at constant L, fixed 20 iterations, no
  trig on the mapping path. This is the same geometry as the W3C-standardized CSS
  Color 4 §13 gamut-mapping algorithm (chroma reduction in Oklch at constant
  lightness and hue); our fixed-iteration bisection (chroma precision 2⁻²⁰) is
  better for determinism than CSS's ΔE-threshold termination. Considered and
  rejected: CSS's MINDE step (accept a channel-clip when ΔEok < 2) — trades back
  hue exactness and adds a discontinuity; CUSP projection in JzAzBz/ICtCp —
  useful when tone and gamut are mapped jointly, unnecessary here because the
  tone map has already bounded every channel, and both spaces put a PQ
  nonlinearity (transcendental-heavy) on the mapping path.
- The viewport WGSL mirrors the CPU operators (`map_to_gamut` and `tonemap` in
  `shader.wgsl`), receiving every matrix from `focale_core::color` constants. The
  contract is perceptual fidelity, not bit-identity (GPU filtering and precision
  differ). No automated CPU-vs-WGSL parity test exists — accepted risk, reviewed
  on any operator change.
- **Deferred (post-v1, keep the seam):** soft-proofing and print intent. The
  colour module is structured so a proofing transform can be inserted before the
  display transform later; the feature is not built.

## 9. Export

- **HARD:** support SDR and HDR output. HDR uses the full capability of each
  format (PQ/HLG transfer, wide gamut, gain maps where the format supports them).
  The wide-gamut working space is mapped into whatever target the user selects.
- **Export codecs** (all licenses verified AGPL-compatible, `[HARD-LICENSE]`;
  encoders run single-threaded with pinned settings so output bytes are
  reproducible, `[HARD-DET]`):
  - TIFF 16-bit: `tiff` (MIT) — the designated hand-off format, with embedded ICC.
  - PNG: `png` (MIT/Apache) — 16-bit, cICP chunk for HDR (PQ) + ICC for SDR.
  - JPEG: `jpeg-encoder` (MIT/Apache) — 8-bit + ICC.
  - JPEG XL: `jpegxl-rs` (GPL-3.0-or-later bindings over BSD-3 libjxl) — 16-bit,
    lossless option, HDR (PQ/HLG). License compatibility: **AGPLv3 §13** grants
    the AGPL-covered work (Focale) permission to link/combine with GPLv3-licensed
    works and convey the result, with the GPLv3 part remaining under GPLv3;
    GPLv3 §13 is the mirror permission on the GPL side.
  - AVIF: `rav1e` (BSD-2) + `avif-serialize` (BSD-3) — 8/10/12-bit per the
    recipe, CICP-signaled PQ/HLG and wide gamut. (Adobe RGB has no H.273 code
    point and is rejected for AVIF.)
- Watchlist (mid-2026): `jxl-oxide` remains decode-only; Imazen's pure-Rust
  `jxl-encoder` 0.3.x (AGPL-3.0-only OR commercial) is pre-1.0 with unverified
  HDR signaling — watch, don't adopt; `ravif` wraps the same rav1e pair but hides
  the threading/CICP knobs determinism requires — direct use is correct; `gamut`
  0.3 is still 8-bit SDR TIFF/AVIF/WebP only (no JXL/PNG/JPEG/16-bit/HDR/ICC), so
  it cannot serve export; revisit as it matures.
- Gain-map export is deferred: the seam is kept in the export-recipe schema (a
  recipe carries an optional `gain_map` block, rejected at execution in v1).

## 10. Preview & compute scheduling

- **One implementation of the pipeline math.** Decode happens once per image; the
  result is immediately box-downscaled to a preview base (long edge ≤ 2560 px)
  and the full-resolution buffer dropped. Every slider change re-runs the CPU
  pipeline on that base; v1 ships without per-stage caching (the seam for it is
  the preview scheduler). The GPU does exactly one thing: the colour-managed blit
  (working→display) plus zoom/pan sampling (§8).
- **Rationale:** the GPU preview must stay perceptually faithful to the CPU path
  forever, across every pipeline version. Duplicating eleven stages in WGSL
  doubles every algorithm and every version freeze. **Honesty note:** the
  <100 ms slider-to-screen figure (§12 targets) is a *budget, not a
  measurement* — no benchmark exists yet; instrumentation and a reproducible
  benchmark are tracked in issue #11. If profiling falsifies the single-pipeline
  design, the seam is the preview scheduler, not the stage code.
- Preview quality: fit-to-window renders on a mip of the demosaiced image; 1:1
  zoom renders the visible tile at full resolution.
- A priority job scheduler owns all background work per opened file: interactive
  preview > thumbnail/filmstrip > export queue > **idle** work.
- The v1 suggestion engine is a stub implementing the full v2 contract: it runs
  when the file's queue goes idle (or immediately on demand), produces per-slider
  `Suggestion { stage, param, value }` proposals, and the UI renders
  accept / tweak / ignore affordances. The stub returns "no suggestions"; the
  scheduling, plumbing, and UI ship in v1.

## 11. Application model (normative)

- **Session model (HARD):** browsing is strictly one directory at a time. Opening
  a directory shows a filmstrip of its raws (thumbnails, flags/ratings from
  sidecars). No recursion, no collections, no database. The directory view builds
  its entire index by scanning sidecar live-index blocks — file names and
  directory shape carry no meaning.
- **Editor:** single-image view with the fixed pipeline presented as ordered
  panels matching §3 stage order. No panel reordering. Redundant/duplicate
  controls are forbidden — one way to do each thing.
- **Batch (HARD):** (a) copy settings from one image and paste to a
  multi-selection; (b) multi-select in the filmstrip while previewing one frame —
  edits save to every selected file's sidecar. Plus a background export queue for
  multi-image export.
- **Status bar (HARD):** persistent, keyed fields including at minimum: active
  rendering gamut, pipeline version of the open file (with the older-version
  warning and the explicit "upgrade to current pipeline" action — the only
  operation that re-stamps a sidecar's version), image colour info under cursor,
  zoom, and warnings (e.g. missing optics metadata).
- **AI-suggestion hook:** v1 ships no suggestion model, but the UI and compute
  scheduler implement the intended behavior as a stub (§10).

## 12. Platform, stack & targets

- **Targets (HARD):** macOS (Apple silicon) and Linux/Wayland, first-class. The
  stack must not preclude Windows/X11, but no effort is spent on them. Current
  reality: Linux is exercised by CI on both architectures; macOS has no CI and no
  platform-specific code yet — tracked in issue #10.
- **GUI stack:** `winit` + `wgpu` + `egui`. Rationale: the image viewport must be
  a custom colour-managed render pass under our control (§8), which rules out
  webview stacks; egui rides the same wgpu surface for panels and keeps the whole
  app in Rust. UI chrome does not require colour precision; the viewport shader
  does.
- **Preview performance target:** slider-to-screen update < 100 ms at
  fit-to-window zoom on a base Apple-silicon Mac; full-resolution CPU export may
  be slower — correct beats fast on the export path. (Unmeasured; issue #11.)

## 13. Verification

Deliverables map:

| Deliverable | Where |
| --- | --- |
| Architecture decisions with rationale | this document |
| Sidecar schema doc + deterministic-encoding golden tests | `docs/sidecar-schema.md`; `focale-sidecar/tests/golden.rs` (committed `canonical.fcl`, double-serialize and map-order-permutation byte equality) |
| Determinism CI across x86_64 + aarch64 | `.github/workflows/determinism.yml` (renders `synthetic.dng` + `determinism.fcl` in every format on both arches and diffs hashes); in-process double-encode guard in `focale-cli/tests/determinism.rs` |
| Pipeline-version regression suite | `focale-core/tests/pipeline.rs` (`rich_edit_matches_frozen_golden` — frozen hash; already caught one real glibc transcendental divergence, fixed by routing through `focale_core::math`/libm) |
| Colour tests (working-space → display/export values per gamut/format) | `focale-core/src/color/*` (48 reference-value tests: matrices re-derived in f64, transfer round-trips, PQ/HLG anchors, Oklab, per-gamut mapping) + `focale-export/tests/export.rs` numeric probes (sRGB 16-bit value, PQ 203-nit value, cICP payloads, per-gamut encodes) |
| Mask parity checklist (§6) as integration tests | `focale-core/tests/masks.rs` (28 tests: every shape, every op, feather/density, groups, determinism) + `focale-segment` unit/integration tests (subject/sky/background/object/person + parts) |
| 20-in-30 workflow possible end-to-end | tooling shipped: keyboard culling, multi-select edit broadcast, copy/paste settings, background export queue; final timing validation tracked on issue #1 |

Determinism CI:

- `focale-cli render <raw> <sidecar>` is the canonical export entry point.
- CI job matrix: `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64) render
  the committed fixture set and compare SHA-256 of output bytes; any divergence
  fails.
- Golden-file suites: (a) sidecar bytes for a canonical edit state, (b) frozen
  sidecars + frozen output hashes per pipeline version (regression), (c) colour
  transform vectors per gamut/format.
