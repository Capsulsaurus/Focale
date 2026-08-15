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
- Reality of the v1 decode stack: rawshift parses **no** optics metadata from
  ARW (the correction arrays sit in SubIFD tags it does not decode — see the
  parameter model below) and only DNG `GainMap` opcodes on its internal DNG
  path. Consequently the stage emits the mandated visible warning ("no optics
  metadata; stage skipped") for affected files. Tracked upstream as rawshift
  [#63](https://github.com/visualcommons/rawshift/issues/63), which carries
  the tag list, the layout traps and the fixture values; that issue is the
  gate on issue #7 and confirmed still absent on rawshift `master`
  (2026-08-15).
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
- **Optical centre:** every radial form is measured about a centre expressed
  in the normalized pre-geometry frame ([sidecar](sidecar.md) §4). It is taken
  from the source when the source states one (DNG opcodes carry it); otherwise
  it defaults to the frame centre `[0.5, 0.5]`. It is never inferred from image
  content — that would be content-adaptive and break `[HARD-VER]`. The resolved
  value is recorded ([sidecar](sidecar.md) §5.15), so a later change of default
  cannot alter an existing edit.
- **Distortion:** the DNG rectilinear warp radial polynomial
  `r_src = r·(k0 + k1·r² + k2·r⁴ + k3·r⁶)` about the optical centre.
- **Lateral CA:** per-channel scale on the same radial polynomial (red and blue
  planes warp with their own coefficients relative to green).
- **Sampled radial knots** as an alternate radial form for all three
  corrections. This is the shape of Sony's embedded correction data, and the
  details below are **verified against the reference implementations**
  (darktable `8b02137d`, RawTherapee `039b9b89`, both read 2026-07-20) rather
  than inferred; landscape in [rnd/lens-database.md](../rnd/lens-database.md).

  - **Where the data lives.** `0x7032` vignetting (`int16s`, `Count` 17),
    `0x7035` CA (`Count` 33), `0x7037` distortion (`Count` 17). These are
    **not MakerNote tags**: they are defined in ExifTool's main EXIF table
    (`Image::ExifTool::Exif::Main`, group **SubIFD**) and read from the raw
    IFD as `Exif.SubImage1.*`. Sony's MakerNotes carry a *separate* set of
    correction arrays at `0x064a`/`0x066a`/`0x06ca` with formats
    `int16s[16]`/`int16s[32]`/`int16s[16]` — **no leading count element**.
    Reading the MakerNote arrays with SubIFD indexing (or vice versa) shifts
    every coefficient by one; the first element of the SubIFD arrays is the
    knot count `n`, and coefficients begin at index 1 (blue CA at `n+1`).
  - **Knot count is body-dependent**, not fixed at 16: Sony's own
    `DistortionCorrParamsNumber` tag enumerates `11 (APS-C)` and
    `16 (Full-frame)`. Read `n` from the array; never assume it.
  - **Knot positions** are `r_i = (i + 0.5) / (n − 1)` over a radius
    normalized to 1.0 at the image corner — **not** evenly spaced from 0 to
    the corner. For `n = 16` the first knot sits at `r ≈ 0.033` and the last
    at `r ≈ 1.033`, so both the image centre and the extreme corners fall
    outside the knot span and are governed by endpoint behaviour. Endpoint
    handling is therefore part of the specification, not an implementation
    detail: **clamp to the terminal knot value at both ends**, pinned per
    pipeline version.
  - **Interpolation is linear between knots**, endpoint-clamped, pinned per
    pipeline version. This differs from the tone curve's monotone cubic
    ([sidecar](sidecar.md) §5.5) and the difference is deliberate. The
    interpolant is not a free choice here: the knots are *measured data
    authored by the camera maker against a specific reconstruction*, and
    linear interpolation is the only reconstruction the two independent
    consumers of this data implement (`_interpolate_linear_spline`,
    darktable `src/iop/lens.cc:2003-2023`). A smoother interpolant would be
    equally deterministic and *less correct* — determinism is satisfied by
    pinning any interpolant, but agreement with the data's author is what
    makes the correction right. Pinning is still what makes the form as
    deterministic as the polynomial form (`[HARD-DET]`); "spline" alone would
    not be a specification.
  - **Decode model** (darktable `_init_coeffs_md_v2`, `src/iop/lens.cc:2194`):
    distortion `d[i]·2⁻¹⁴ + 1` as a radial scale factor; lateral CA
    `ca_r[i]·2⁻²¹ + 1` and `ca_b[i]·2⁻²¹ + 1` multiplied onto the distortion
    factor, green untouched; vignetting `2^(0.5 − 2^(v[i]·2⁻¹³ − 1))` as a
    multiplicative gain. These constants are **reverse-engineered and
    undocumented upstream** — ExifTool names the tags but supplies no
    `ValueConv`, so they are structurally decoded and semantically
    unspecified. Treat them as empirical: version this model as
    reverse-engineered from the outset, because refining a constant later is
    an output-changing algorithm change (`[HARD-VER]`).
  - **Open question, must be resolved empirically before issue #7 ships.**
    darktable and RawTherapee **disagree on the vignetting sign convention**:
    darktable computes the gain above, RawTherapee computes its *reciprocal*
    (`rtengine/lensmetadata.cc:253`) and both then multiply it into the raw
    data. One applies gain where the other attenuates. RawTherapee is a
    direct port of darktable (stated in-source at `lensmetadata.cc:160-163`),
    so this is not two independent derivations disagreeing — it is one
    lineage with a likely sign bug, and their agreement elsewhere is **common
    ancestry, not corroboration**. Resolve against a real ARW before pinning
    the vignetting direction.

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
  recorded parameters (`[HARD-DET]`, `[HARD-VER]`). The schema home for those
  recorded parameters is [sidecar](sidecar.md) §5.15, which specifies both
  radial forms and the provenance string this trait returns.
