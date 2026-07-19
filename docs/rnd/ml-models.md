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

1. **Models are separate assets, never part of the AGPL work.** Weights are
   downloaded post-install by the user, live in the user data directory, and
   are optional. This keeps `[HARD-LICENSE]` clean: AGPL §7 forbids imposing
   further restrictions on the covered work, so restricted weights must never
   be combined into it.
2. **License passthrough.** Mirrored models ship with their full license text
   and required notices; the manifest's `notices` field is displayed at fetch
   time.
3. **OpenRAIL-class models (none shipped today):** RAIL licenses permit
   mirroring but require the use-restriction attachment to travel with every
   copy as enforceable terms, plus a license copy to every recipient. They are
   not open source by OSD standards (field-of-use restrictions). Policy: RAIL
   weights may be offered via the manifest as upstream-fetch or mirrored
   *with full passthrough*, always as separate restricted assets, never
   presented as free software. Whether to accept any RAIL model at all is a
   per-model decision recorded here.
4. **Dataset-terms risk is documented, not hidden.** Where a model's training
   data carries research-only terms, the manifest entry and this doc record
   it (see the inventory). The prevailing analysis (multiple sources,
   2026-07): whether dataset click-through terms reach trained weights is
   **legally unsettled** — weight copyrightability is itself open, contract
   privity binds the trainer rather than downstream recipients, and the
   CelebA/CelebAMask-HQ agreements (unlike e.g. Waymo's, which names model
   weights explicitly) contain no model-reaching clause. Focale's position:
   redistribute only what upstream distributes under a permissive grant,
   record the residual risk, and keep weights out of the AGPL work (rule 1).

## Shipped model inventory (v1, verified 2026-07-19)

| Model | Artifact(s) | License chain | Risk notes | Hosting |
| --- | --- | --- | --- | --- |
| MobileSAM (subject/object click-to-select) | `mobile_sam_image_encoder.onnx`, `sam_mask_decoder_single.onnx` | MobileSAM repo Apache-2.0; teacher SAM weights explicitly Apache-2.0; Acly ONNX export repo tagged MIT | The MIT relabel of a mechanical conversion of Apache-2.0 weights is doubtful — **treat the ONNX files as Apache-2.0 and carry SAM/MobileSAM attribution**. MobileSAM checkpoints have no weight-specific statement (repo license governs). | Mirror-eligible (Apache-2.0 terms honored) |
| BiSeNet face parsing (person parts) | `face_parsing_resnet18.onnx` | yakhyo/face-parsing MIT ← zllrunning/face-parsing.PyTorch MIT | **Load-bearing caveat:** trained on CelebAMask-HQ, whose agreement is "non-commercial research purposes only" and bars redistributing "any portion of derived data" (models/weights not mentioned; "derived data" undefined). Unsettled-law analysis above; the whole ecosystem redistributes on the MIT basis; risk judged low for a local-only AGPL project **but it is a judgment call, recorded here**. The upstream MIT grant is only as good as the trainers' authority to make it. | Mirror-eligible with this caveat recorded; revisit if a cleanly-licensed face parser appears |
| U²-Net saliency (subject/background) | `u2net.onnx` | U-2-Net repo Apache-2.0; rembg (ONNX export host) MIT | Trained on DUTS-TR, which sources images from ImageNet (research-only terms) — same unsettled dataset-terms class as above, one remove further. | Mirror-eligible, caveat recorded |
| U²-Net sky segmentation | `skyseg.onnx` | Upstream xiongzhu666 repo MIT (LICENSE file present); HF rehost tagged MIT but **has no LICENSE file** | Training data unstated. Action item for mirroring: capture the upstream LICENSE text into our mirror rather than relying on the HF tag. | Mirror-eligible after LICENSE capture |

`scripts/fetch-models.sh` is the current (pre-manifest) form of this mechanism:
pinned URLs + sha256 + per-model license comments. Migrating it to read the
manifest is part of the v2 model-distribution work; its license comments defer
to this doc for the full analysis.

## Roadmap

Placements are owned by [scope](../scope.md); details here.

- **`v2 (committed)` — AI-suggested slider values.** The v1 stub already ships
  the full contract ([preview](../subsystems/preview.md)). Direction:
  neural-guided optimization toward a professional-corpus target; training
  details TBD. Off the export path (suggestions become ordinary recorded
  sidecar values), so runtime role 1 applies.
- **`v2 (committed)` — neural denoise & sharpen.** New pipeline-versioned
  stages; **blocked on the deterministic runtime**
  ([inference.md](inference.md) role 2 gate).
- **`v2 (committed)` — mask-model upgrades.** BiRefNet (code MIT; fp32 ONNX
  ~973 MB, fp16 ~490 MB; lite ~224 MB, lite-fp16 ~115 MB — lite variants are
  the evaluation target) for matting; a multi-person-capable parser for issue
  #8; an iris-capable model for issue #9. Cheap by design: masks resolve into
  the sidecar, so model swaps never break old edits
  ([masks](../subsystems/masks.md)). Verify BiRefNet training-data terms
  before committing.
- **`high-priority future` — super-resolution.** A new pipeline-versioned
  stage at the **end of the pipeline (after detail/denoise)**, available in
  **preview and export** — not an export-only option. On the export path ⇒
  role 2 gate applies. Model candidate survey happens when the runtime gate
  clears.
- **`eventually` — auto-culling assist.** Sharpness/closed-eye/duplicate
  scoring feeding the culling workflow ([app](../subsystems/app.md));
  creation-time only.
- **`eventually` — edit-style learning.** Personalizing the suggestion engine
  from the user's own sidecar history; local-only training (`[HARD-LOCAL]`).
- **`never` — local semantic search:** rationale in [scope](../scope.md#never).
