# Raw decode & demosaic

Pipeline stage 1 ([pipeline](pipeline.md)): raw file → linear camera RGB f32.
Owning code: `focale-core/src/decode`. Governing invariants: `[HARD-DET]`,
`[HARD-VER]`, `[HARD-RUST]` ([invariants](../invariants.md)).

- **Decode crate** (`v1 (shipped)`): `rawshift-image` with `arw` + `dng` features
  (the crate's two "stabilizing" formats). Supported today: Sony
  lossless-compressed ARW (compression type 7 — A7 IV / A7R V / A1 generation
  bodies) and DNG. Uncompressed and lossy ARW decode will arrive upstream; we
  surface a clear per-file error until then (`v1 (gap, issue #12)`). Initial
  camera support target: Sony ARW first-class; other makes as the decode crate
  allows.
- **Path:** `decode_raw()` (u16 CFA) → black-level subtract → demosaic →
  normalize `u16 / white_level` to **linear camera RGB f32**.
- **Demosaic is pinned per pipeline version** to `Bayer(Amaze)` — never `Auto`,
  which is content-adaptive and would break `[HARD-VER]`. rawshift offers AMaZE,
  RCD, and LMMSE for Bayer (plus Markesteijn for X-Trans); AMaZE is RawTherapee's
  Bayer default and the reference for low-ISO detail recovery, while RCD trades a
  little fine detail for fewer overshoot artifacts and speed — but speed is
  irrelevant on the export path ("correct beats fast") and preview runs on the
  downscaled base, so AMaZE's quality edge wins. LMMSE is superior on
  very-high-ISO frames; exposing the demosaic algorithm as a *recorded sidecar
  parameter* (never content-adaptive) would stay fully deterministic —
  `eventually` ([scope](../scope.md#eventually)). rawshift's row-parallel
  demosaic writes disjoint rows from immutable input, so it is deterministic
  under any thread count.
- **Camera colour matrix** (XYZ→camera, DNG convention) comes from rawshift's
  bundled per-model database with dual-illuminant CCT interpolation; DNG files
  use their embedded `ColorMatrix1/2`. The `gamut` crate is not used at the
  decode stage (it is a codec library); it was evaluated and its current versions
  lack what export needs ([export](export.md)).
- **Optics metadata** parsed (or not) at decode time is the input to the optical
  corrections stage; the presence struct and its limits are specified in
  [optics](optics.md).
