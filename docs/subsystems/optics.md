# Optical corrections

Pipeline stage 2 ([pipeline](pipeline.md)): vignetting, lateral chromatic
aberration, geometric distortion. Owning code: `focale-core/src/decode`
(metadata presence) and `focale-core/src/params/optics.rs` (stage toggles); the
correction math itself is `v1 (gap, issue #7)`. Governing invariants:
`[HARD-DET]`, `[HARD-VER]` ([invariants](../invariants.md)).

This document owns the **stage behaviour, the correction parameter model, and the
`CorrectionSource` seam**. The v2 external profile database that will feed the
seam is designed in [rnd/lens-database.md](../rnd/lens-database.md), which
consumes — never restates — the contract defined here.

## v1 source policy (HARD)

- v1 source is exclusively embedded metadata (**HARD**). If metadata is absent
  and nothing can be inferred, the stage emits a visible warning in the UI and is
  skipped — **never guess, never fail**.
- Reality of the v1 decode stack: rawshift 0.1.1 parses **no** optics metadata
  from ARW (Sony stores it in undecoded MakerNote tags) and only DNG `GainMap`
  opcodes on its internal DNG path. Consequently the stage emits the mandated
  visible warning ("no optics metadata; stage skipped") for affected files.
- Shipped seam (`v1 (shipped)`): the `OpticsMetadata` presence struct
  (`focale-core/src/decode`), the `OpticsParams` stage toggles (inert until
  metadata exists), and the warning plumbing through to the status bar
  ([app](app.md)).

## Correction parameter model (normative for issue #7)

One internal parameter model, regardless of source. Sources (embedded metadata
now, profile database later) are adapters that map **into** this model; the stage
math never knows where parameters came from.

The model follows the DNG opcode parameterization — the natural fit because
embedded DNG opcodes are the v1 source, and every other candidate source
(lensfun-style polynomials, future measured profiles) maps onto it losslessly or
with a documented fit:

- **Vignetting:** a radial gain function (DNG `FixVignetteRadial` form) or a
  sampled gain map (DNG `GainMap`), applied as a per-pixel multiply in linear
  camera RGB. No resampling is involved, so it applies even when the geometric
  corrections are off.
- **Distortion:** the DNG rectilinear warp radial polynomial
  `r_src = r·(k0 + k1·r² + k2·r⁴ + k3·r⁶)` about the optical centre.
- **Lateral CA:** per-channel scale on the same radial polynomial (red and blue
  planes warp with their own coefficients relative to green).
- **Sampled radial splines** as an alternate radial form for all three
  corrections: Sony bodies store corrections as evenly-spaced radial spline
  knots in reverse-engineered MakerNote tags (`0x7032` vignetting, `0x7035`
  CA, `0x7037` distortion — decoded today by ExifTool/darktable/RawTherapee;
  landscape in [rnd/lens-database.md](../rnd/lens-database.md)). Spline
  evaluation with pinned interpolation is deterministic exactly like the
  polynomial form; this is the expected shape of ARW embedded metadata once
  rawshift exposes it.

## Application order & determinism (normative for issue #7)

1. Vignetting gain first (pure per-pixel multiply — commutes with nothing
   downstream, cheapest first, valid even without geometry metadata).
2. Distortion + lateral CA as **one combined inverse-mapped warp per channel** —
   a single resampling pass, never two chained resamples (each resample loses
   detail and compounds error).
3. The interpolation kernel is pinned per pipeline version (v1: bicubic
   Catmull-Rom over f32, individually rounded ops, fixed row-major traversal) —
   content-independent and bit-deterministic per `[HARD-DET]`.
4. Coordinates outside the source frame resolve to edge-clamp; the stage never
   changes image dimensions (crop decisions belong to geometry, stage 9).

## The `CorrectionSource` seam (v2)

`v2 (committed)` — tracked with issue #7. A trait in `focale-core`:

```text
CorrectionSource::lookup(camera_model, lens_id, focal_mm, aperture, focus_distance)
    -> Option<CorrectionParams>   // + provenance string for the status bar
```

- v1 ships exactly one implementation: `EmbeddedMetadata` (the adapter over what
  the decode stage surfaced).
- The v2 profile database ([rnd/lens-database.md](../rnd/lens-database.md)) is a
  second implementation. Resolution order and user override UI arrive with it.
- Applied correction parameters are recorded in the sidecar at edit time, so
  exports never re-query a source — same pattern as resolved AI masks
  ([masks](masks.md)): lookups are creation-time, the export path replays
  recorded parameters (`[HARD-DET]`, `[HARD-VER]`).
