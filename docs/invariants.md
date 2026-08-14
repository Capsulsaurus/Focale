# Invariants — the HARD principles

This document is the single source of truth for Focale's non-negotiable
requirements. Each carries a stable ID (e.g. `[HARD-DET]`) that code comments and
other documents cite; the IDs are permanent. No other document restates these
definitions — summaries elsewhere (README, CLAUDE.md/AGENTS.md) are derived from
this file and say so.

Violating a HARD principle is never a valid trade-off. A change that cannot
satisfy them all is rejected or redesigned.

1. **`[HARD-DET]` Determinism.** Identical (raw file + sidecar + pipeline version)
   inputs produce **bit-identical export output on any machine**. The export path
   is CPU-only, uses no non-deterministic parallel reductions, no `fast-math`,
   fixed iteration orders, and pinned algorithm versions. The GPU is used **only**
   for interactive preview and must be perceptually faithful to the CPU path, but
   bit-identity is not required of the preview.
2. **`[HARD-VER]` Permanent pipeline versioning.** Every sidecar records the
   pipeline version that created it. Newer software must recreate the identical
   export from older sidecars forever. Changing any algorithm's output requires
   introducing a new pipeline version while retaining the old implementation. No
   exceptions, no deprecation. The same permanent-compatibility rule applies to
   the sidecar schema.
3. **`[HARD-LOCAL]` Local-only.** All computation, including all future AI, runs
   on the user's machine. **No network calls in the application** — the binary
   the user runs has no networking code path at all, not merely no telemetry.
   The boundary is the application: a sanctioned, user-invoked script may
   download assets (model weights, [rnd/ml-models.md](rnd/ml-models.md)),
   because the user chooses to run it, it runs outside the app, and the app
   works — degraded but honestly — without it. An in-app "download models"
   button would violate this principle even though the bytes are identical.
4. **`[HARD-LICENSE]` License.** AGPL-3.0. Chosen because the network-use
   clause (§13) is what keeps a hosted derivative from becoming a proprietary
   fork of a local-only tool; GPL-3.0 would leave that open, and a permissive
   licence would leave the whole work open to enclosure. The cost is accepted
   deliberately: AGPL deters some commercial adoption and constrains
   dependencies to AGPL-compatible licences. External contributions require a
   CLA assigning rights to the project author, so relicensing remains possible
   without tracing every contributor. All dependencies must be
   AGPL-compatible; verify before adding any crate or model weights. Model
   weights are handled as separate assets, never combined into the covered
   work ([rnd/ml-models.md](rnd/ml-models.md)).
5. **`[HARD-RUST]` Rust core.** All processing logic is Rust. Existing crates for
   raw decode, codecs, and math are preferred over reimplementation; do not reject
   a crate for immaturity alone.
6. **`[HARD-FS]` Filesystem is the source of truth.** All persistent state lives
   in per-image sidecars and plain files next to the images. No catalogue
   database, no import step, no proprietary store — deleting Focale loses
   nothing, and nothing about a user's work is held hostage to the application
   (no vendor lock-in). Derived caches (thumbnails, directory indexes) are
   permitted but must be reconstructible from the filesystem alone and never
   consulted as an authority. A centralized DB-driven catalogue is strictly
   prohibited, permanently ([scope](scope.md)).

Where the principles bind hardest:

- `[HARD-DET]` + `[HARD-VER]` shape the whole [processing pipeline](subsystems/pipeline.md)
  and the [sidecar format](subsystems/sidecar.md).
- `[HARD-LOCAL]` shapes [ML model distribution](rnd/ml-models.md) (the app never
  downloads; a sanctioned script does).
- `[HARD-LICENSE]` gates every dependency and every model weight
  ([export codecs](subsystems/export.md), [ML models](rnd/ml-models.md)).
- `[HARD-FS]` shapes the [application model](subsystems/app.md) (directory
  sessions, sidecar-scan indexing, XMP interop).
