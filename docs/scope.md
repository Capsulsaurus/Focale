# Scope — what Focale is, and what ships when

This document owns two things: the **product definition** and the **status
vocabulary + placement** of every feature. Subsystem docs tag their own features
with the same vocabulary and carry the detail; this file is the master index and
links there rather than restating. If a feature's placement here and a tag in a
subsystem doc ever disagree, this file wins and the other is a bug.

## Product definition

A desktop raw photo developer for experienced photographers who want deliverable
results fast. One fixed, correctness-ordered processing pipeline; creative
flexibility expressed through masks and (v2) AI-suggested parameter values — never
through panel sprawl or reorderable node graphs.

**Success metric (acceptance bar):** an experienced photographer takes 20 culled
raw images and finishes editing them in 30 minutes, at output quality matching what
they would produce in the raw editor of their choice. (Validation run tracked on
issue [#1](https://github.com/Capsulsaurus/Focale/issues/1).)

Remaining v1 delivery gaps are tracked as sub-issues of
[#1 (V1 Roadmap)](https://github.com/Capsulsaurus/Focale/issues/1); docs note the
relevant issue number wherever a described seam is not yet implemented.

## Status vocabulary

Every feature in every doc carries exactly one of these tags:

| Tag | Meaning |
| --- | --- |
| `v1 (shipped)` | Implemented and released in v1. |
| `v1 (gap, issue #N)` | Part of the v1 contract, not yet implemented; tracked. |
| `v2 (committed)` | Will be built; the v1 code keeps a deliberate seam for it. |
| `high-priority future` | Committed direction with a named blocker; scheduled as soon as the blocker clears. |
| `eventually` | Wanted, unscheduled, no seam obligation beyond not precluding it. |
| `demand-driven` | Built only if user demand materializes; recorded so the decision isn't relitigated. |
| `never` | Out of scope permanently, with rationale. Reversing requires amending this document. |

## v1

The v1 feature set, per subsystem (each doc carries the detail and its own
shipped/gap tags): [pipeline](subsystems/pipeline.md),
[decode](subsystems/decode.md) (Sony lossless ARW + DNG),
[optics from embedded metadata](subsystems/optics.md),
[masks at full parity incl. AI segmentation](subsystems/masks.md),
[colour management](subsystems/color.md), [SDR+HDR export](subsystems/export.md),
[preview & scheduling](subsystems/preview.md),
[directory sessions, culling, batch](subsystems/app.md),
[sidecar format](subsystems/sidecar.md).

Known v1 gaps (all sub-issues of #1): optical-correction math (#7), wide-gamut
display output (#6, #10), uncompressed/lossy ARW decode (#12, upstream),
multi-person parsing (#8) and sclera/iris separation (#9) — model-capability
limits, preview benchmark & instrumentation (#11), macOS CI (#10), 20-in-30
validation run (#1).

## v2 (committed)

- **AI-suggested slider values** — the v1 stub ships the full scheduling and UI
  contract ([preview](subsystems/preview.md), [ML roadmap](rnd/ml-models.md)).
- **Lens measurement kit + open correction-profile database** — separate project;
  integration seam in [optics](subsystems/optics.md); design in
  [rnd/lens-database.md](rnd/lens-database.md).
- **Neural denoise and sharpening** — new pipeline-versioned stages; blocked on
  the deterministic inference runtime ([rnd/inference.md](rnd/inference.md)).
- **Culling interop via XMP** — one-way derived XMP mirror + one-time import;
  `.fcl` stays the source of truth ([app](subsystems/app.md#culling--xmp-interop)).
- **Gain-map export** — the export-recipe schema already carries the seam
  ([export](subsystems/export.md)).
- **BiRefNet-family matting upgrade** and other mask-model upgrades — cheap by
  design, since masks resolve into the sidecar ([masks](subsystems/masks.md)).

## High-priority future

- **Super-resolution** — a new pipeline-versioned stage at the end of the
  pipeline (after detail/denoise), available in **preview and export**, not an
  export-only option. Because it runs on the export path it inherits
  `[HARD-DET]`, so it is blocked on the deterministic inference runtime
  ([rnd/inference.md](rnd/inference.md); roadmap detail in
  [rnd/ml-models.md](rnd/ml-models.md)).

## Eventually

- **Stacking/merge** — HDR bracket merge, panorama stitch, focus stacking;
  produces a new DNG that the normal pipeline then edits (keeps the one-pipeline
  invariant intact).
- **Print & soft-proof output** — printer-profile soft-proofing and a print
  intent; the colour module keeps the proofing-transform seam
  ([color](subsystems/color.md)).
- **Auto-culling assist** — sharpness/closed-eye/duplicate scoring to pre-rank a
  shoot; off the export path, so no determinism burden
  ([rnd/ml-models.md](rnd/ml-models.md)).
- **Edit-style learning** — personalize the suggestion engine from the user's own
  sidecar history; local-only training ([rnd/ml-models.md](rnd/ml-models.md)).
- **Content-aware / generative retouch** — beyond v1's clone/heal
  ([masks](subsystems/masks.md), [ML roadmap](rnd/ml-models.md)).
- **Recorded demosaic choice** — exposing LMMSE (superior at very high ISO) as a
  sidecar-recorded parameter, never content-adaptive
  ([decode](subsystems/decode.md)).

## Potential extensions (given demand)

Recorded so the door stays open without committing effort:

- **Tethered capture** — shoot-to-folder from a connected camera; fits the
  directory model ([app](subsystems/app.md)); Sony-first if built.
- **Windows as a first-class target** — the stack must not preclude it (that part
  is HARD, [platform](subsystems/platform.md)); investment happens only on
  demand. Packaging decisions are pre-recorded in
  [platform](subsystems/platform.md#distribution--packaging).
- **Unsupervised batch HDR→SDR conversion** — would motivate a BT.2446-based
  operator as a new pipeline version ([color](subsystems/color.md)).

## Never

Each entry names its rationale; reversing any of these requires amending this
document, not just building the feature.

- **Cataloging / DAM / centralized database** — prohibited by `[HARD-FS]`
  ([invariants](invariants.md)). Users pair Focale with external DAM tools; the
  only index is rebuilt by scanning sidecars.
- **Cloud anything, telemetry, accounts** — `[HARD-LOCAL]` and product identity.
- **Heavy retouching** — users export 16-bit TIFF to Photoshop-class tools; a
  raw developer that grows a compositor loses its shape.
- **Reorderable pipeline / node graphs** — the fixed correctness-ordered
  pipeline is the product ([pipeline](subsystems/pipeline.md)).
- **Redundant/duplicate UI controls** — one way to do each thing
  ([app](subsystems/app.md)).
- **Local semantic search** ("find photos of X" via embeddings) — sits exactly on
  the `[HARD-FS]` boundary: even as a reconstructible cache it drags in an
  embedding store, a query UI, and DAM expectations. That is a catalogue by
  another name; external tools do this well.
- **Plugin / scripting API** — third-party code injecting into the pipeline is
  irreconcilable with `[HARD-DET]` (unversioned external operators break
  bit-permanence), erodes one-way-to-do-each-thing, and creates an AGPL boundary
  question with proprietary plugins.
- **Linux/X11 colour management** — not a scope decision but a platform
  impossibility; recorded in [platform](subsystems/platform.md) and
  [color](subsystems/color.md).
