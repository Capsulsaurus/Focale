# PRD — Focale: A Deterministic, Guided Raw Developer

Status: v1 specification. Audience: implementing agent. Anything marked **HARD** is
non-negotiable. Anything marked *(agent's choice)* is delegated, but the choice must be
documented in `docs/architecture.md` with rationale.

---

## 1. Product definition

A desktop raw photo developer for experienced photographers who want deliverable
results fast. One fixed, correctness-ordered processing pipeline; creative flexibility
expressed through masks and (v2) AI-suggested parameter values — never through panel
sprawl or reorderable node graphs.

**Success metric (acceptance bar):** an experienced photographer takes 20 culled raw
images and finishes editing them in 30 minutes, at output quality matching what they
would produce in the raw editor of their choice.

**Explicit non-goals:** cataloging/DAM (users pair with external tools), heavy
retouching (users export 16-bit TIFF to Photoshop-class tools), cloud anything,
telemetry, accounts.

## 2. Non-negotiable principles

1. **HARD — Determinism.** Identical (raw file + sidecar + pipeline version) inputs
   produce **bit-identical export output on any machine**. The export path is CPU-only,
   uses no non-deterministic parallel reductions, no `fast-math`, fixed iteration
   orders, and pinned algorithm versions. The GPU is used **only** for interactive
   preview and must be perceptually faithful to the CPU path, but bit-identity is not
   required of the preview.
2. **HARD — Permanent pipeline versioning.** Every sidecar records the pipeline version
   that created it. Newer software must recreate the identical export from older
   sidecars forever. Changing any algorithm's output requires introducing a new
   pipeline version while retaining the old implementation. No exceptions, no
   deprecation.
3. **HARD — Local-only.** All computation, including all future AI, runs on the user's
   machine. No network calls in the application.
4. **HARD — License.** AGPL-3.0. External contributions require a CLA assigning rights
   to the project author. All dependencies must be AGPL-compatible; verify before
   adding any crate or model weights.
5. **HARD — Rust core.** All processing logic is Rust. Existing crates for raw decode,
   codecs, and math are preferred over reimplementation; do not reject a crate for
   immaturity alone.

## 3. Processing pipeline

**HARD — the stage order is fixed** and identical for preview and export. Users cannot
reorder stages; they can only enable/disable and parameterize them. Order:

1. **Raw decode** — demosaic to linear camera RGB, f32. *(strictly use gamut and rawshift crate in Rust.)* Initial camera support target: Sony ARW first-class; other
   makes as the decode crate allows.
2. **Optical corrections** — vignetting, chromatic aberration, distortion, applied from
   **embedded manufacturer metadata in the raw file** (HARD: this is the primary and
   only correction source in v1). If metadata is absent and nothing can be inferred,
   emit a visible **warning** in the UI and skip the stage — never guess, never fail.
   Architecture must leave a seam for a v2 external profile database.
3. **White balance** and camera-to-working-space transform.
4. **Global tone** — exposure, contrast, highlights/shadows/whites/blacks, tone curve
   (parametric + point curve).
5. **Global colour** — HSL per-band, colour grading (shadows/midtones/highlights
   wheels), vibrance/saturation.
6. **Local adjustments** — any subset of stages 4–5 parameters applied through masks
   (§4).
7. **Detail** — capture sharpening (unsharp/deconvolution) and conventional noise
   reduction (luma/chroma). Non-neural in v1; architecture must accommodate v2 neural
   replacements as new pipeline-versioned stages.
8. **Retouch** — heal and dust-spot removal (clone/heal brush). Content-aware
   inpainting is out of scope for v1.
9. **Geometry** — crop, rotate, perspective. *(agent's choice: whether geometry is
   composited earlier in preview for UX; export math must match preview framing.)*
10. **Finishing** — post-crop vignette, grain.
11. **Output transform** — working space → target colour space + tone mapping (§5).

**Working space (HARD):** linear Rec.2020 primaries, f32, unbounded (values may exceed
[0,1] until output transform).

## 4. Masks — "Adobe parity" defined

v1 must ship all of the following mask types and operations. This list is the
definition of parity; nothing less.

**Geometric:** brush (size/feather/flow, eraser), linear gradient, radial gradient.
**Range:** luminance range, colour range (sampled, with tolerance/falloff).
**AI-segmented (local ONNX models, HARD: local inference only):** subject, sky,
background, objects (click/brush-to-select), people — with per-person components: face
skin, body skin, hair, eyebrows, eyes (sclera + iris/pupil), lips, teeth, clothing.
**Operations:** any mask combinable via add / subtract / intersect / invert; per-mask
feather and density (max-opacity) controls; masks nest into named groups.

*(agent's choice: segmentation model family — e.g. SAM-variant + portrait parser —
and runtime, e.g. `ort`. Models must be redistributable under AGPL-compatible terms
and run acceptably on an 8 GB-VRAM GPU and Apple-silicon Macs; CPU fallback permitted
but may be slow.)*

Mask rasterization participates in determinism: mask evaluation in the export path is
CPU, bit-identical. Neural segmentation output is stored **resolved into the sidecar**
(vectorized or compressed bitmap, agent's choice) at creation time, so exports never
re-run a model — this is what keeps AI masks compatible with the determinism guarantee.

## 5. Colour management

- **Preview (HARD):** the app must render colour-managed on macOS and Wayland Linux.
  The image viewport is drawn by our own wgpu shader performing working-space →
  display-space conversion. Assume professional users on ~Display P3 D65 hardware, but
  never hard-code: query the OS/compositor for the surface colour space (macOS:
  tagged `CAMetalLayer`; Wayland: `wp_color_management_v1` where available, else
  user-set display profile with sRGB default).
- **HARD:** the GUI has a user-selectable **active rendering gamut** (sRGB, Display P3,
  Adobe RGB) and the currently active gamut is always visible as a key in the status
  bar (§7).
- **Export (HARD):** support SDR and HDR output. HDR uses the full capability of each
  format (PQ/HLG transfer, wide gamut, gain maps where the format supports them). Our
  wide-gamut working space is mapped into whatever target the user selects.
  *(agent's choice: gamut-mapping and HDR→SDR tone-mapping operators — documented and
  pipeline-versioned.)*
- **Deferred (post-v1, keep the seam):** soft-proofing and print intent. The colour
  module must be structured so a proofing transform can be inserted before the display
  transform later; do not build the feature now.

## 6. Sidecar format

- **HARD:** one sidecar per image, **CBOR with RFC 8949 §4.2 Core Deterministic
  Encoding** — identical edits always serialize to identical bytes. File name:
  `<image-filename>.<ext>.fcl` *(Focale extension — document it)*.
  Stored alongside the image; the raw file is never modified.
- Contents: schema version, pipeline version, full parameter set for every stage,
  resolved masks, retouch strokes, crop/geometry, export recipes, and a **live-index
  metadata block** (rating/flag/label, capture-time cache, thumbnail hash) sufficient
  for the directory view (§7) to build its index by scanning sidecars alone — file
  names and directory shape carry no meaning.
- **HARD:** schema is forward-versioned with the same permanent-compatibility rule as
  the pipeline. Publish the schema in-repo as documentation.

## 7. Application UX

- **Session model (HARD):** browsing is strictly one directory at a time. Opening a
  directory shows a filmstrip of its raws (thumbnails, flags/ratings from sidecars).
  No recursion, no collections, no database.
- **Editor:** single-image view with the fixed pipeline presented as ordered panels
  matching §3 stage order. No panel reordering. Redundant/duplicate controls are
  forbidden — one way to do each thing.
- **Batch (HARD, UI-level feature):** (a) copy settings from one image and paste to a
  multi-selection; (b) multi-select in the filmstrip while previewing one frame — edits
  save to every selected file's sidecar. Plus a background export queue for
  multi-image export.
- **Status bar (HARD):** persistent, keyed fields including at minimum: active
  rendering gamut, pipeline version of the open file, image colour info under cursor,
  zoom, and warnings (e.g. missing optics metadata).
- **AI-suggestion hook:** v1 ships no suggestion model, but the UI and compute
  scheduler must implement the intended behavior as a stub: suggestions compute
  **lazily after all other work for the opened file is idle**, or immediately
  on-demand, and surface as accept/tweak/ignore proposals on sliders. Wire the
  scheduling and UI affordance now; the model arrives in v2.

## 8. Platform & stack

- **Targets (HARD):** macOS (Apple silicon) and Linux/Wayland, first-class. The stack
  must not preclude Windows/X11, but do not spend effort on them.
- **GUI stack (decided):** `winit` + `wgpu` + `egui`. Rationale: the image viewport
  must be a custom colour-managed render pass under our control (§5), which rules out
  webview stacks; egui rides the same wgpu surface for panels and keeps the whole app
  in Rust. UI chrome does not require colour-precision; the viewport shader does.
- **Preview performance target:** slider-to-screen update < 100 ms at fit-to-window
  zoom on a base Apple-silicon Mac; full-resolution CPU export may be slower — correct
  beats fast on the export path.
- **Export codecs:** TIFF (16-bit, the designated hand-off format), JPEG XL, AVIF,
  PNG, JPEG — each driven to the best of the format's capability (bit depth, gamut,
  HDR). *(agent's choice of crates; verify AGPL compatibility.)*

## 9. Deferred to v2 (do not build, do not block)

- AI-suggested slider values (GAN-discriminated, neural-guided non-linear optimization
  toward a professional-corpus target; training details TBD).
- Lens measurement kit + open correction-profile database (separate project;
  integration seam required per §3.2).
- Neural denoise and sharpening (new pipeline-versioned stages).
- Soft-proofing / print intent.
- Content-aware / generative retouch.

## 10. Deliverables checklist for the implementing agent

- [ ] `docs/architecture.md` recording every *(agent's choice)* with rationale
- [ ] Sidecar schema doc + golden-file tests proving deterministic encoding
- [ ] Determinism CI: export the same (raw + sidecar) on two different architectures
      (x86_64 + aarch64) and diff bytes
- [ ] Pipeline-version regression suite: frozen sidecars + frozen expected outputs
- [ ] Colour tests: known working-space values → expected display/export values per
      gamut, per format
- [ ] Mask parity checklist (§4) as integration tests
- [ ] The 20-images-in-30-minutes workflow demonstrably possible end-to-end