# R&D: Open lens-correction profile database

The design for Focale's `v2 (committed)` lens measurement kit + open
correction-profile database ([scope](../scope.md#v2-committed)) — a separate
project with a hard integration seam into Focale. It consumes the
`CorrectionSource` contract and correction parameter model defined in
[optics](../subsystems/optics.md); nothing here redefines them. Landscape
findings verified 2026-07-19 unless noted.

## Why a new database — the landscape and its restrictions

**lensfun** (the only established open database):

- Licensing: library LGPL-3.0; **database CC BY-SA 3.0** — still 3.0, which
  (unlike 4.0) does not clearly license EU sui generis database rights, and
  its share-alike is viral for derived databases. Consequence, load-bearing:
  **a fresh permissively-licensed database cannot bootstrap from lensfun
  data** — importing its XML would impose BY-SA on ours.
- Maintenance: last release v0.3.4 (2023-07); the project lead declared the
  project "more or less stalled" and asked the community to take over (2019).
  The git database still receives contributions (Sony entries as recent as
  2026-06), so apps consuming release tarballs run ~3 years behind git.
- Models: distortion poly3 / poly5 / ptlens / ACM; vignetting `pa` (+ ACM);
  TCA linear / poly3 / ACM. **Distortion and TCA are parameterized by focal
  length only — no focus-distance dimension exists in the schema** (vignetting
  alone records focus distance), so close-focus distortion variation is
  unmodeled *(inference from the schema, not a quoted limitation)*.
- Provenance/QA: community-contributed calibrations with no documented review
  process or contributor licensing agreement; quality is uneven and
  unauditable. ACM-model profiles are explicitly not accepted into the
  official database (the `lensfun-convert-lcp` tool is user-side only).

**Adobe LCP:** profile *data* ships only inside Adobe products; no
redistribution grant exists (RawTherapee tells users to extract profiles from
Adobe's own installers). The profile-creator tooling is discontinued. But the
**model math is usable**: the Adobe rectilinear camera model appears in the DNG
specification's opcodes (`WarpRectilinear`, `FixVignetteRadial`) under Adobe's
royalty-free DNG-spec patent grant — implementable without touching Adobe data.
This is exactly the parameter family [optics](../subsystems/optics.md) pins.

**What the incumbents actually do:** darktable = lensfun + embedded-metadata
corrections (Sony/Fuji/DNG, added because modern mirrorless files carry their
own correction data); RawTherapee = lensfun + user-supplied LCP +
embedded-metadata (5.11+). Sony ARW correction data lives in reverse-engineered
MakerNote tags — `0x7032` vignetting (int16×17), `0x7035` CA (int16×33),
`0x7037` distortion (int16×17): radial **spline knots**, evenly spaced
centre→corner, decoded by ExifTool (Artistic/GPL) and applied by
darktable/RawTherapee (GPL-3). The format knowledge itself is unencumbered
reverse engineering. This is the upstream path for Focale's own v1
embedded-metadata source (rawshift, issue #12/#7).

**Other options:** DxO modules — proprietary, no access. Gyroflow's
`lens_profiles` — **CC0-1.0, actively maintained (v37, 2026-01)** — the
strongest precedent that a CC0 community calibration database with in-app
contribution works; but it's geometry-only video calibration, not usable
directly. No ODbL lens database exists; no lensfun fork has momentum.

## Design

### Data license: CC0-1.0

Deciding factors: the stated goal is a database usable by *everyone* —
proprietary editors included — to maximize contributions and adoption; CC0 is
the only class with zero compliance friction for every consumer (waives
copyright *and* database rights), and it is what the one thriving post-2020
community calibration DB (Gyroflow) chose. Rejected: CC BY-SA 4.0 — permits
proprietary *use* but share-alike on derived/merged databases plus an
AGPL-asymmetry (BY-SA 4.0 is one-way compatible with GPLv3 only, not AGPL);
ODbL — attribution + share-alike plumbing for every consumer and essentially
untested in this space. Contributions require a CC0 dedication from the
contributor at submission time (the lensfun lesson: no licensing agreement ⇒
frozen license forever).

### Profile format

- **Correction models = exactly the parameter family in
  [optics](../subsystems/optics.md):** the DNG-opcode radial polynomial family
  plus sampled radial splines. Measured data is stored as fitted parameters
  *and* the residual fit error; a profile that cannot meet a stated residual
  bound is rejected, not shipped degraded.
- **Dimensions:** per (camera body, lens, focal length, aperture, **focus
  distance**) — fixing lensfun's missing close-focus dimension for distortion
  and TCA, with interpolation rules pinned in the format spec (so application
  stays deterministic per `[HARD-DET]` once parameters are recorded into a
  sidecar).
- **Provenance is first-class:** every profile records the measurement-kit
  version, capture setup, submitter, and QC status. No anonymous unauditable
  numbers — the lensfun provenance gap is the direct motivation.
- Format is versioned with the same permanent-compatibility discipline as the
  sidecar schema (`[HARD-VER]` culture, [sidecar](../subsystems/sidecar.md)
  §3.4 as the template).

### Measurement kit

Goals: a repeatable target + capture protocol + fitting tool an interested
photographer can run without lab equipment; deterministic fitting (same
captures ⇒ same profile); output is a submission-ready profile with residuals
and provenance attached. The kit, not the reviewer, does the quality
gatekeeping: submissions failing residual/coverage bounds are rejected
mechanically. (Detailed kit design is future work in this doc.)

### Contribution & QC pipeline

Git-backed database repo; submissions as PRs from the kit's output; CI
validates schema, residual bounds, and provenance completeness; human review is
for plausibility, not numbers. Releases are tagged snapshots — apps pin a
release, like any dependency.

### Distribution & integration

- Distributed via the same manifest + hash-pinning mechanism as ML models
  ([ml-models.md](ml-models.md)) — one sanctioned fetch path, `[HARD-LOCAL]`
  intact, CC0 makes mirroring trivial.
- Integration into Focale is a second `CorrectionSource` implementation
  ([optics](../subsystems/optics.md)); lookups happen at edit time and the
  chosen parameters are **recorded into the sidecar**, so exports never
  consult the database and profile updates never change old edits
  (`[HARD-DET]`, `[HARD-VER]`).
- Because the database is CC0 and self-contained, proprietary editors can
  adopt it wholesale — which is the point: the database's value grows with its
  consumer base, and Focale keeps no lock-in (`[HARD-FS]` spirit applied to
  community data).
