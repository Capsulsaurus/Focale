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
- Maintenance **(corrected 2026-07-20 — "stalled" was wrong)**: lensfun is
  *actively curated but release-frozen*. ~290 commits from 20+ committers in
  the last 12 months, commits as recent as 2026-06, not archived. But the last
  stable release is **v0.3.4 (2023-07-12)**, and the "0.4 beta" tag that
  distros package (`v0.3.95`) resolves to a commit from **2018-06-29**. The
  practical consequence is the one that matters for us: the *database* keeps
  growing while the *library and schema* do not move, so the schema gap below
  will not close on its own.
- Models: distortion poly3 / poly5 / ptlens / ACM; vignetting `pa` (+ ACM);
  TCA linear / poly3 / ACM. Shipped usage: distortion ptlens 5,538 / poly3 872
  / poly5 5; TCA poly3 3,753 / linear 5; vignetting pa 29,445; **ACM zero** —
  the parser accepts it but no shipped profile uses it, and the DTD/XSD do not
  even list it. Coverage: 1,558 lenses, 1,045 cameras.
- **Focus distance — the recorded inference is now VERIFIED as fact
  (2026-07-20).** Distortion and TCA are parameterized by focal length only;
  vignetting alone carries `focal`/`aperture`/`distance`. Proven at four
  levels: the DTD/XSD (`CalibrationDistortionType` and `CalibrationTcaType`
  declare no `distance` attribute), the C++ API itself
  (`lfLens::InterpolateDistortion(crop, focal, …)` and `InterpolateTCA(crop,
  focal, …)` versus `InterpolateVignetting(crop, focal, aperture, distance,
  …)` — focus distance is absent from the *function signature*, so it cannot
  be plumbed through without an ABI break), the shipped database (zero
  `<distortion … distance=` and zero `<tca … distance=` occurrences across 56
  XML files, against 29,445 vignetting entries that all carry it), and
  upstream's own acknowledgement (discussion #1640, open since 2022, reporting
  a Sigma 35mm whose blue-channel TCA displacement swings from ≈+1 px at
  infinity to ≈−4 px at close focus on 24 MP — agreed as needed, never fixed).
  - The `lensfun-convert-lcp` tool is the clinching evidence: Adobe LCP files
    *do* carry `FocusDistance` per entry, and the converter **discards it**,
    keeping one entry per focal length. The axis exists in commercial data and
    lensfun structurally cannot store it.
  - Even where the dimension does exist it is barely sampled: of 709 lenses
    with vignetting data, **568 (80%) have exactly two distinct focus
    distances**, and the calibration tutorial tells contributors that
    "shooting at infinity is sufficient".
- Design lesson taken from this, for our own format: lensfun made `distance`
  `use="required"` on vignetting, which forces contributors to write `1000`
  (an infinity proxy) as a lie-by-convention. **Ours should make focus
  distance optional with an explicit "calibrated at infinity" marker** —
  sparse honestly-labelled data beats dense mandatory placeholders. The
  exponential capture burden is real and is exactly why lensfun's contributors
  default to infinity-only.
- Model-choice note for `[HARD-DET]`: prefer **poly5**. Both ptlens and poly3
  carry an implicit focal-length rescaling because `d = 1−a−b−c ≠ 1`, which
  upstream describes as PTLens being "defined in a bad way"; poly5 is clean.
- Provenance/QA: community-contributed calibrations with no documented review
  process or contributor licensing agreement; quality is uneven and
  unauditable. ACM-model profiles are explicitly not accepted into the
  official database (the `lensfun-convert-lcp` tool is user-side only).

**Adobe LCP:** profile *data* ships only inside Adobe products; no
redistribution grant exists (RawTherapee tells users to extract profiles from
Adobe's own installers). The profile-creator tooling is effectively
discontinued — Adobe's user-submission profile database is gone, and the Lens
Profile Downloader has been unavailable since ~2018, so **there is currently no
active crowd-sourced profile database outside lensfun**. Note LCP *is*
focus-distance-aware (its entries carry `FocusDistance`), which is direct
evidence that focus-distance-dependent distortion and TCA is an established,
commercially-shipped concept rather than a speculative feature of our design.

The **model math is usable**: the Adobe rectilinear camera model appears in the
DNG specification's opcodes (`WarpRectilinear`, `FixVignetteRadial`), and this
is the parameter family [optics](../subsystems/optics.md) pins.

> **⚠ Unverified claim — do not rely on this until it is checked
> (flagged 2026-07-20).** This paragraph previously asserted that the opcodes
> are available "under Adobe's royalty-free DNG-spec patent grant —
> implementable without touching Adobe data". **That grant text has never been
> read by this project.** Research could not retrieve `dng_patent_license.pdf`,
> so there is no quoted operative wording, no confirmation that it is
> royalty-free, no reading of its conditions or any defensive-termination
> clause, and therefore **no basis for an AGPL-3.0 compatibility assessment**.
> The claim may well be correct — it is the widely-held understanding — but
> `[HARD-LICENSE]` requires verifying licence compatibility *before* adoption,
> and this is the load-bearing licence claim under the entire correction-model
> choice. Resolve against the actual licence text before issue #7 ships.

**`WarpRectilinear2` (DNG 1.6, opcode 14) is worth evaluating** when that
licence question clears: it extends the radial polynomial to odd *and* even
powers through r¹⁴, adds an optional division form, and — most relevant to
`[HARD-DET]` — **declares an explicit valid-radius range and requires the
function be one-to-one within it**, rather than assuming global invertibility
as lensfun does. It also encodes lateral CA through per-plane coefficient sets
and is defined as a *reverse* mapping, so no render-time inversion is needed.
That is a cleaner contract than any other open correction model surveyed.

**What the incumbents actually do:** darktable = lensfun + embedded-metadata
corrections (shipped in 4.2, Dec 2022, because modern mirrorless files carry
their own correction data); RawTherapee = lensfun + user-supplied LCP +
embedded-metadata, **ported from darktable 4.6 and stated as such in-source**
(so the two are one lineage, not independent corroboration — see the vignetting
sign discrepancy in [optics](../subsystems/optics.md)). Both read these tags via
**exiv2, not LibRaw**: LibRaw 0.22.0 contains no reference to them at all and
leaves correction entirely to the client.

The tag details previously summarised here (locations, counts, knot spacing,
interpolation) were **partly wrong and have been corrected in
[optics](../subsystems/optics.md)**, which owns the parameter model — most
importantly, these are *SubIFD* tags rather than MakerNote tags, and the knots
are neither evenly spaced from centre to corner nor cubic-interpolated. This
document does not restate them. The format knowledge itself is unencumbered
reverse engineering. This is the upstream path for Focale's own v1
embedded-metadata source (rawshift, issue #12/#7).

**Other options:** DxO modules — proprietary, no access; note their fallback
behaviour is *silent* degradation (the sharpness palette simply disappears and
a generic unsharp mask is substituted), which is strictly worse than the
visible-warning policy [optics](../subsystems/optics.md) mandates. ART is the
better model to study: it reads embedded manufacturer correction metadata and
Adobe LCP files directly, which is closer to our `CorrectionSource` seam.
Gyroflow's `lens_profiles` was recorded here as "CC0-1.0, actively maintained
(v37, 2026-01)" — **that licence, the "v37" designation, and the date are all
UNVERIFIED** and should not be relied on; research could not confirm them.
Its calibrator is OpenCV-based (chessboard video, fisheye/standard models),
which makes it very likely **geometry-only with no vignetting or TCA data** —
also unconfirmed, and the question that decides whether it is a usable seed
source at all. No ODbL lens database exists; no lensfun fork has momentum.

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
  ([ml-models.md](ml-models.md), which owns that mechanism and records what is
  built today versus targeted) — one sanctioned fetch path, `[HARD-LOCAL]`
  intact, CC0 makes mirroring trivial. Profiles are a second consumer of the
  manifest, not a second mechanism; nothing about the fetch path is redefined
  here.
- Integration into Focale is a second `CorrectionSource` implementation
  ([optics](../subsystems/optics.md)); lookups happen at edit time and the
  chosen parameters are **recorded into the sidecar** — concretely, into the
  `ResolvedCorrection` structure at [sidecar](../subsystems/sidecar.md) §5.15,
  whose two radial forms this format must therefore map onto. Exports never
  consult the database and profile updates never change old edits
  (`[HARD-DET]`, `[HARD-VER]`).
- Because the database is CC0 and self-contained, proprietary editors can
  adopt it wholesale — which is the point: the database's value grows with its
  consumer base, and Focale keeps no lock-in (`[HARD-FS]` spirit applied to
  community data).
