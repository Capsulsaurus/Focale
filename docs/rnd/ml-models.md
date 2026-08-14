# R&D: ML models — distribution, licensing, roadmap

How Focale distributes ML model weights, the compliance rules that govern every
model, the verified license status of the shipped set, and the ML feature
roadmap. Runtime selection is owned by [inference.md](inference.md). Governing
invariants: `[HARD-LOCAL]`, `[HARD-LICENSE]` ([invariants](../invariants.md)).
License findings verified 2026-07-19 unless noted; **none of this is legal
advice — it records the project's researched position.**

## Distribution mechanism (uniform for every model)

**Decided: one manifest, split hosting.** The same mechanism serves every
model, present and future; only the hosting location varies, driven by what
each license permits.

- **Model manifest** (in-repo, versioned): one entry per model artifact —
  `id`, `version`, `file name`, `sha256`, `size`, `license` (SPDX or
  `LicenseRef-`), `source` (upstream provenance URL), `host` (download URL),
  and a `notices` field for attribution/provenance text the fetch step must
  display. `scripts/fetch-models.sh` is the manifest's consumer and the **one
  sanctioned download path** — the app itself never touches the network
  (`[HARD-LOCAL]`); it only loads models from the user data directory and
  shows a "model not installed" affordance.
- **Split hosting:** models whose licenses permit redistribution are mirrored
  on a Focale-owned release host (stability; upstream HF/GitHub deletions
  can't break installs). Models whose terms disallow or complicate rehosting
  are fetched from upstream, hash-pinned. Same manifest shape, same script,
  same UX either way.
- Hash pinning is load-bearing: a model swap upstream cannot silently change
  what users run, and creation-time inference results are reproducible enough
  to debug (bit-determinism is *not* required off the export path —
  [inference.md](inference.md)).

## Compliance rules

1. **Models are treated as separate assets, never part of the AGPL work.**
   Weights are downloaded post-install by the user, live in the user data
   directory, and are optional; the app degrades to a "model not installed"
   affordance without them. This keeps `[HARD-LICENSE]` clean: GPLv3 §10
   forbids imposing further restrictions on the covered work, and §7 makes
   any non-permissive additional term a "further restriction", so restricted
   weights must never be combined into it. Where contradictory conditions
   would prevent simultaneous compliance, §12 says the work may not be
   conveyed at all — and a defective AGPL grant downstream exposes the project
   to claims from **its own contributors**, not merely a model licensor. That
   is why the separation is architectural rather than a matter of packaging
   convenience.

   **Status of this rule, stated honestly (2026-07-20):** this is a *reasoned
   position*, not a verified legal conclusion, and the previous flat assertion
   ("Models **are** separate assets") claimed more certainty than the project
   can substantiate. The doctrinal case is genuinely strong for Focale's
   specific architecture — opaque data consumed at runtime, never linked or
   compiled in, fetched by a separate script, optional — which is about as
   clean a mere-aggregation posture as exists. But research could not retrieve
   an FSF, SFC, SFLC, Debian, or Fedora position addressing ML weights and
   copyleft specifically, nor verify what comparable projects (GIMP, Krita,
   darktable, digiKam, Blender) actually do. **Open follow-up:** substantiate
   rule 1 against GPLv3 §5 mere-aggregation text, the GPL FAQ, and distro AI
   policies. Until then it stands as the project's considered position, and it
   is the *conservative* choice regardless — the rule's practical effect is to
   admit fewer models, not more.
2. **License passthrough.** Mirrored models ship with their full license text
   and required notices; the manifest's `notices` field is displayed at fetch
   time.
3. **OpenRAIL-class models: rejected outright. No RAIL-licensed weights are
   accepted, mirrored, or offered via the manifest.** *(Tightened 2026-07-20
   from a per-model decision; the previous "mirror with full passthrough"
   policy understated what the licence actually demands.)* Three
   primary-source reasons, any one sufficient:
   - **§4(a) is a licensing act, not a documentation duty.** It requires the
     use-based restrictions to "be included as an **enforceable provision by
     You** in any type of legal agreement… governing the use and/or
     distribution of the Model", plus a copy of the licence to every
     recipient. An AGPL project that ships an enforceable Attachment A has
     imposed a further restriction — precisely what GPLv3 §10 forbids.
   - **§7 conflicts with our invariants directly.** The licensor "reserves the
     right to restrict (remotely or otherwise) usage of the Model" and to
     "update the Model through electronic means", and requires that "You shall
     undertake reasonable efforts to use the latest version of the Model."
     That is irreconcilable with `[HARD-VER]` (permanent versioning) and
     `[HARD-DET]` (frozen bytes forever), and sits badly beside `[HARD-LOCAL]`.
   - **§1(e) captures our own pipeline.** "Derivatives" includes any model
     "created or initialized by transfer of patterns of the weights,
     parameters, activations or output" — so ONNX conversion and quantization
     produce Derivatives. There is no laundering step.

   On status, cite the **FSF**, which is more authoritative than an OSD
   appeal: gnu.org's licence list names **BigScience Open RAIL-M** in the
   *Nonfree Software Licenses* section — "these are nonfree licenses, because
   they deny freedom 0", and the obey-all-laws clause "can have the effect of
   denying freedoms 2 and 3." No RAIL licence is OSI-approved, and OpenRAIL
   fails the OSAID "use for any purpose without asking permission" criterion
   outright. Note also that RAIL adoption is in decline (major adopters have
   left, mostly for *more* restrictive bespoke licences) and the steward has
   published nothing since March 2024 — this is not a standard converging on
   acceptability.
4. **Dataset-terms risk is documented, not hidden.** Where a model's training
   data carries research-only terms, the manifest entry and this doc record
   it (see the inventory). Whether dataset click-through terms reach trained
   weights is **legally unsettled** — weight copyrightability is itself open
   and contract privity binds the trainer rather than downstream recipients.

   **Correction 2026-07-20 — the previous wording overstated our own case.**
   It claimed the CelebA/CelebAMask-HQ agreements "contain no model-reaching
   clause". That is true of the three *Dataset Agreement* bullets, whose
   distribution restriction is scoped to "any portion of the CelebAMask-HQ
   **dataset**". But the same README carries a fourth restriction under a
   separate heading: *"The use of **this software** is RESTRICTED to
   non-commercial research and educational purposes."* That clause is not
   dataset-scoped, and it is the one a rights-holder would actually point at.
   The contrast with Waymo (which defines "Derivative IP" to include
   "architectures, weights, and parameters") is about **explicitness, not
   restrictiveness** — and Waymo in fact *permits* publishing trained weights.

   One piece of evidence in the other direction, previously unrecorded:
   CelebAMask-HQ's own README links `zllrunning/face-parsing.PyTorch` under
   "Related Projects using CelebAMask-HQ". The dataset authors know of the
   derived model, link it approvingly, and have not objected in ~6 years. That
   is not a licence, but it is real evidence of acquiescence.

   **The state of the law, for calibration.** The strongest recent holding is
   favourable: *Getty Images v Stability AI* [2025] EWHC 2863 (Ch) ¶600 held
   that "the model weights are not themselves an infringing copy and they do
   not store an infringing copy", calling the contrary submission "entirely
   misconceived". But it does not settle the question — Getty never argued
   weights contain copies, and the experts agreed memorization is real. Other
   courts diverge: *Bartz v. Anthropic* (2025) **assumed** weights retain
   "compressed copies" and reached fair use anyway; *Andersen v. Stability AI*
   (2024) held that works represented "as algorithmic or mathematical
   representations" is "not an impediment to the claim". Fair use is a defence
   you fund, not a property line — and it fit Anthropic partly because the use
   was "spectacularly" transformative, a poor analogy for shipping a
   segmentation model to end users.

   Focale's position, unchanged but now better grounded: redistribute only
   what upstream distributes under a permissive grant, admit no
   non-commercial or research-only terms, record per-model training-data
   provenance, prefer upstream-fetch over self-mirroring where terms are
   murky, and keep weights out of the AGPL work (rule 1). This posture does
   not depend on any of the four judicial positions resolving our way.

## Shipped model inventory (v1, verified 2026-07-19)

The BiSeNet face parser below is the one entry whose position is a judgment
call rather than a clean grant, and it is **accepted as final for v1** — the
analysis in rule 4 is the reasoning, and re-deciding it per release would be
churn without new information. It carries a standing revisit trigger instead:
re-open the decision if a cleanly-licensed face parser of comparable quality
appears, if any rights-holder objects to redistribution, or if the unsettled
dataset-terms question above is settled by case law or a licence change.

**Trigger status 2026-07-20: partially fired — v1 unchanged, v2 changed.** A
clean-licensed candidate now exists, but it is not a drop-in replacement:

- **EasyPortrait** (SberDevices) is the only surveyed candidate clearing both
  the code and dataset bars. Its dataset licence is a CC-BY-SA-derived
  "public license with attribution and conditions reserved" with **no
  non-commercial clause**, and validated ONNX exports exist. **But it parses
  8 classes and has no hair class** (vs CelebAMask-HQ's 19) — losing hair is
  a material capability regression for portrait masking, so it cannot
  replace BiSeNet alone.
- The only fully-clean *combination* found is **EasyPortrait (face parts) +
  MediaPipe Multiclass Segmenter (hair, genuinely Apache-2.0)**, at the cost
  of two models and an ONNX/TFLite split. Recorded as the clean-provenance
  path for v2, not a v1 action.
- **Everything else is CelebAMask-HQ laundered through a permissive tag.**
  Every MIT/Apache face-parsing model surveyed on HuggingFace traces back to
  it. LaPa is self-contradictory (non-commercial README against an Apache-2.0
  LICENSE file), and Microsoft FaceSynthetics is explicitly model-reaching
  ("Artificial intelligence models trained on Data… are Results", barred from
  commercial offerings).
- **Open item before any adoption:** EasyPortrait's repo root has no LICENSE
  file — only the dataset licence PDF in a `license/` directory — so whether
  the *checkpoints* are covered is genuinely unresolved. Ask upstream before
  committing. Also note a third-party ONNX repack is tagged `apache-2.0`,
  which is almost certainly wrong; do not propagate that tag into the manifest.

Until a single clean 19-class parser appears or a rights-holder objects, the
recorded v1 position stands.

| Model | Artifact(s) | License chain | Risk notes | Hosting |
| --- | --- | --- | --- | --- |
| MobileSAM (subject/object click-to-select) | `mobile_sam_image_encoder.onnx`, `sam_mask_decoder_single.onnx` | MobileSAM repo Apache-2.0 (real LICENSE file); teacher SAM weights explicitly Apache-2.0 — Meta's README licenses "**the model**", not merely the code; Acly ONNX export repo tagged MIT | **Confirmed 2026-07-20: treat the ONNX files as Apache-2.0 and carry SAM/MobileSAM attribution.** The MIT relabel is not defensible — `torch.onnx.export` is transcoding, not authorship, and Apache→MIT silently strips the §4 NOTICE duty and the §3 patent grant, which only a copyright holder could do. The mislabel also *originates upstream* of Acly (the `dhkim2810` weights repo carries a bare `license: mit` tag with no LICENSE file, against the canonical repo's Apache-2.0). Neither ONNX repo has a LICENSE file — both are HF card tags only. SA-1B's research-only terms do **not** contaminate us: Meta deliberately split research-only images from Apache-2.0 checkpoints. **Actions:** ship an actual NOTICE (§4(d)); cite the GitHub LICENSE as authority, never the HF card; and prefer **re-exporting from `ChaoningZhang/MobileSAM` with its own `export_models.py`**, which drops both murky hops from the chain for the cost of one fetch-script step. | Mirror-eligible (Apache-2.0 terms honored, NOTICE shipped) |
| BiSeNet face parsing (person parts) | `face_parsing_resnet18.onnx` | yakhyo/face-parsing MIT ← zllrunning/face-parsing.PyTorch MIT (both real LICENSE files, verified present) | **Load-bearing caveat, sharpened 2026-07-20.** Trained on CelebAMask-HQ ("non-commercial research purposes only"; bars commercial exploitation of "any portion of derived data", undefined; **plus a separate, non-dataset-scoped clause restricting "this software" to non-commercial research** — see rule 4, which previously overstated our case). Second, narrower problem: **the MIT grant may not reach the artifact we ship.** Both LICENSE files are standard MIT scoped to "the Software"; `yakhyo/face-parsing` has an *empty* `weights/` directory and distributes the ONNX via GitHub Releases, outside the repo tree, with no separate weights grant. So the clean chain describes a grant over *code*. Counterweight: the dataset authors link the derived model approvingly and have not objected in ~6 years. Risk still judged acceptable for a local-only AGPL project **but it is a judgment call, and a closer one than previously recorded**. | Mirror-eligible with these caveats recorded; see the revisit-trigger note below — it has now partially fired |
| U²-Net saliency (subject/background) | `u2net.onnx` | U-2-Net repo Apache-2.0; rembg (ONNX export host) MIT | Trained on DUTS-TR, which sources images from ImageNet (research-only terms) — same unsettled dataset-terms class as above, one remove further. | Mirror-eligible, caveat recorded |
| U²-Net sky segmentation | `skyseg.onnx` | Upstream xiongzhu666 repo MIT (LICENSE file present); HF rehost tagged MIT but **has no LICENSE file** | Training data unstated. Action item for mirroring: capture the upstream LICENSE text into our mirror rather than relying on the HF tag. | Mirror-eligible after LICENSE capture |

### Current state vs. target state

The mechanism above is the *target*; be precise about what exists today, since
two other documents ([lens-database.md](lens-database.md),
[platform](../subsystems/platform.md)) already reference the manifest as
though it were built.

| Aspect | Today | Target |
| --- | --- | --- |
| Registry | Pinned URLs + sha256 + licence comments inline in `scripts/fetch-models.sh` | The versioned in-repo manifest described above |
| Consumer | The script itself | The script, reading the manifest |
| Hosting | Upstream URLs only | Split hosting (mirror where licences permit) |
| Notices | Licence comments in the script, deferring here | The `notices` field, displayed at fetch time |

The hash pinning and the single-sanctioned-path property — the two things
`[HARD-LOCAL]` and reproducibility actually rest on — hold in both states;
what the migration buys is one machine-readable registry instead of a shell
script, and a shape the lens-profile database can reuse unchanged. **The
manifest is specified here and nowhere else**: other docs consume it by
reference so the three future asset types cannot drift apart. Migrating is
part of the v2 model-distribution work.

## Roadmap

Placements are owned by [scope](../scope.md); details here.

- **`v2 (committed)` — AI-suggested slider values.** The v1 stub already ships
  the full contract ([preview](../subsystems/preview.md)). Direction:
  neural-guided optimization toward a professional-corpus target; training
  details TBD. Off the export path (suggestions become ordinary recorded
  sidecar values), so runtime role 1 applies.
- **`v2 (committed)` — neural denoise & sharpen.** New pipeline-versioned
  stages; **blocked on the deterministic runtime**
  ([inference.md](inference.md) role 2 gate). The gate is itself a committed
  v2 deliverable ([scope](../scope.md#v2-committed)) — these stages are
  committed *behind* it, and if the gate fails to produce a qualifying
  runtime they do not ship in a degraded form: a neural stage that cannot be
  bit-reproduced across architectures is not shippable at any quality
  (`[HARD-DET]`).
- **`v2 (committed)` — mask-model upgrades.** A multi-person-capable parser
  for issue #8; an iris-capable model for issue #9; a matting upgrade.
  Cheap by design: masks resolve into the sidecar, so model swaps never break
  old edits ([masks](../subsystems/masks.md)).

  **BiRefNet: verified 2026-07-20, and the answer is bad — do not adopt on
  current terms.** The prior note said "verify BiRefNet training-data terms
  before committing"; that verification is done. The code is genuinely MIT
  (real LICENSE file, scoped to "the Software"), and the recorded ONNX sizes
  are accurate to the byte (fp32 972.7 MB, fp16 489.7 MB, lite 224.0 MB,
  lite-fp16 114.5 MB). The chain breaks in two places:
  - **Training data.** The flagship checkpoints are DIS5K-trained, and the
    DIS5K Terms of Use are non-commercial with unusually aggressive
    downstream reach: commercial use "is prohibited **even after copying,
    editing, processing or any operations of this database**", and
    distribution "as it is, or copy, edit, or process this database, in whole
    or in part" is prohibited. If that propagates, a non-commercial condition
    is **flatly incompatible with AGPL-3.0** — it is not on GPLv3 §7's closed
    list of permissible additional terms, so we could not ship it at all.
    (Trap for future readers: GitHub reports the DIS repo as `apache-2.0`;
    that badge covers code and evaluation metrics, not the dataset.)
  - **No clean variant exists on both axes.** The DIS5K-trained flagship at
    least carries an MIT tag; the portrait and lite variants that avoid DIS5K
    carry **no licence declaration at all** (`cardData.license: None`, no
    LICENSE file). One trades a data-provenance problem for a missing-grant
    problem. The `onnx-community` MIT tags are inherited `base_model:`
    metadata from a re-exporter with no authority to license.

  Cheapest next step, given `[HARD-LICENSE]`: ask the author in writing to
  add LICENSE files to the weights repos and state his DIS5K position. Until
  then BiRefNet is not adoptable. **RMBG-2.0 is not an escape hatch** — same
  architecture, explicitly non-commercial. The clean-provenance matting path
  is the EasyPortrait + MediaPipe combination noted in the inventory above.
  Note for evaluation: the P3M-10k / AM-2k / AIM-500 matting datasets *are*
  clean (MIT, no non-commercial clause), so a model trained on those would
  clear this bar.
- **`high-priority future` — super-resolution.** A new pipeline-versioned
  stage at **stage 9.5 — after geometry, before the output transform**
  (placement rationale in [pipeline](../subsystems/pipeline.md)), available in
  **preview and export** — not an export-only option. On the export path ⇒
  role 2 gate applies. Model candidate survey happens when the runtime gate
  clears; note the placement constrains the survey, since the model must
  accept a fully-processed, geometry-final image rather than raw-adjacent
  input as most published SR models assume.
- **`eventually` — auto-culling assist.** Sharpness/closed-eye/duplicate
  scoring feeding the culling workflow ([app](../subsystems/app.md));
  creation-time only.
- **`eventually` — edit-style learning.** Personalizing the suggestion engine
  from the user's own sidecar history; local-only training (`[HARD-LOCAL]`).
- **`never` — local semantic search:** rationale in [scope](../scope.md#never).
