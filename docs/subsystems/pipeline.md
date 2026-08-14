# Processing pipeline

The normative definition of Focale's processing pipeline: the fixed stage order,
the working space, the determinism rules on the export path, and the mechanics of
permanent pipeline versioning. Owning code: `focale-core` (`pipeline`, `image`,
`math`, `params`). Governing invariants: `[HARD-DET]`, `[HARD-VER]`
([invariants](../invariants.md)).

## Stage order (normative)

**HARD (`[HARD-VER]`) — the stage order is fixed** and identical for preview and
export. Users cannot reorder stages; they can only enable/disable and parameterize
them. Order:

1. **Raw decode** — demosaic to linear camera RGB, f32 ([decode](decode.md)).
2. **Optical corrections** — vignetting, chromatic aberration, distortion, from
   embedded raw metadata only in v1 ([optics](optics.md)).
3. **White balance** and camera-to-working-space transform.
4. **Global tone** — exposure, contrast, highlights/shadows/whites/blacks, tone
   curve (parametric + point curve).
5. **Global colour** — HSL per-band, colour grading (shadows/midtones/highlights
   wheels), vibrance/saturation.
6. **Local adjustments** — stage 4–5 parameters applied through masks
   ([masks](masks.md)). The locally-available set is defined by the schema
   ([sidecar](sidecar.md) §5.7), which is narrower than stages 4–5 in v1 and
   widens additively; local adjustments are always deltas over the global
   result, never a second independent pass.
7. **Detail** — capture sharpening (unsharp/deconvolution) and conventional noise
   reduction (luma/chroma). Non-neural in v1; the v2 neural replacements arrive
   as new pipeline-versioned stages (`v2 (committed)`,
   [scope](../scope.md#v2-committed)).
8. **Retouch** — heal and dust-spot removal (clone/heal brush). Content-aware
   inpainting: `eventually` ([scope](../scope.md#eventually)).
9. **Geometry** — crop, rotate, perspective. Applied at this position in both
   preview and export: the viewport simply draws the geometry-stage output, which
   keeps export math and preview framing identical by construction (no earlier
   compositing).
10. **Finishing** — post-crop vignette, grain.
11. **Output transform** — working space → target colour space + tone mapping
    ([color](color.md)), executed by `focale-export` ([export](export.md)).

A future **super-resolution** stage (`high-priority future`,
[scope](../scope.md#high-priority-future)) slots in as **stage 9.5** — after
geometry, before the output transform — as a new pipeline-versioned stage
available in both preview and export.

Why there, rather than earlier or at encode time. Super-resolution
synthesizes detail, and any *spatial* stage that runs after it amplifies what
it invented: sharpening hardens SR artifacts, noise reduction is fed
statistics no sensor produced, and a resample re-filters synthetic edges.
So nothing spatial may follow it, which puts it after detail (7), retouch
(8), and geometry (9) — and makes it the pipeline's last resample, operating
on an image that is already denoised, sharpened, and geometry-final. It is
*not* an encoder-side upscale: the output transform (11) is a per-pixel
colour operation that neither disturbs nor is disturbed by resolution, and
finishing (10) is deliberately after it so grain and vignette are applied at
final output scale rather than being magnified. Placing SR at decode instead
would force every subsequent stage to run at the upscaled resolution for no
quality gain.

## Preview renders the export pipeline (`[HARD-DET]`)

**Preview and export run the same stage set, in the same order, with the same
parameters — the only difference permitted is the resolution of the input
buffer.** A stage may never be preview-only or export-only, and no stage may
branch on whether it is rendering for preview.

This is what makes the viewport an honest proof of the exported file, and it
is why super-resolution is specified as a pipeline stage rather than an export
option: an export-only stage would mean the user never sees what they ship.
The corollary is a performance obligation rather than an escape hatch —
every stage must be affordable at preview-base resolution
([preview](preview.md)), and an export is then the same computation carried
out at full resolution and handed to an encoder.

## Working space

**HARD (`[HARD-DET]`):** linear Rec.2020 primaries, f32, unbounded — values may
exceed [0,1] (and camera colours outside Rec.2020 survive as negative components)
until the output transform.

Rationale for Rec.2020 over the alternatives: it is the working space of
darktable's scene-referred pipeline and matches the linear-primaries philosophy of
Lightroom's internals; ACEScg (AP1) is only marginally wider and buys nothing for
a stills developer that does not interchange with VFX pipelines; ProPhoto/Melissa
has imaginary primaries (physical nonsense values that make per-channel operations
behave unintuitively) and a D50 white point forcing an extra adaptation;
Oklab-style spaces are non-linear and non-radiometric — blurs, resampling, and
blending are physically wrong in them, so Oklab is used at the operator level
instead ([color](color.md)). Decisive extra: every HDR container (PQ/HLG CICP) is
built around Rec.2020, so the HDR export path has an identity primary transform.

## Determinism rules on the export path (`[HARD-DET]`)

CPU only; `rayon` permitted solely for disjoint-row/tile maps (no reductions
across threads); histograms and any whole-image statistics are computed
sequentially in fixed order; no `HashMap` iteration touches pixels; no
`fast-math`, no FMA-dependent algorithms (only individually rounded f32 ops); all
transcendentals route through `focale_core::math` (pure-Rust libm — std float
functions differ across glibc versions).

## Versioning mechanics (`[HARD-VER]`)

Every stage is a pure pipe-filter function `(image, &Params)` keyed by pipeline
version. Version 1 algorithms live in `focale_core::pipeline::v1` and are frozen
at release; changing output requires adding `v2` modules while `v1` stays. A v2
reuses unchanged v1 stage functions directly, so old versions remain re-runnable
forever even when the new version's defaults differ. The GUI and CLI both render
(preview, edit, export) with the sidecar's **stored** pipeline version; only an
explicit user "upgrade" action re-stamps it. The full mechanism — single dispatch
point, frozen per-version module trees, stored-version rendering, and the UI
warning/upgrade flow — is specified in [sidecar](sidecar.md) §3.
