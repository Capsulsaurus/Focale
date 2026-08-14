# Verification

How every normative claim in these docs is enforced by tests and CI.

## Deliverables map

| Deliverable | Where |
| --- | --- |
| Architecture decisions with rationale | this doc tree ([index](README.md)) |
| Sidecar schema doc + deterministic-encoding golden tests | [subsystems/sidecar.md](subsystems/sidecar.md); `focale-sidecar/tests/golden.rs` (committed `canonical.fcl`, double-serialize and map-order-permutation byte equality) |
| Determinism CI across x86_64 + aarch64 | `.github/workflows/determinism.yml` (renders `synthetic.dng` + `determinism.fcl` in every format on both arches and diffs hashes); in-process double-encode guard in `focale-cli/tests/determinism.rs` |
| Pipeline-version regression suite | `focale-core/tests/pipeline.rs` (`rich_edit_matches_frozen_golden` — frozen hash; already caught one real glibc transcendental divergence, fixed by routing through `focale_core::math`/libm) |
| Colour tests (working-space → display/export values per gamut/format) | `focale-core/src/color/*` (48 reference-value tests: matrices re-derived in f64, transfer round-trips, PQ/HLG anchors, Oklab, per-gamut mapping) + `focale-export/tests/export.rs` numeric probes (sRGB 16-bit value, PQ 203-nit value, cICP payloads, per-gamut encodes) |
| Mask parity checklist ([masks](subsystems/masks.md)) as integration tests | `focale-core/tests/masks.rs` (28 tests: every shape, every op, feather/density, groups, determinism) + `focale-segment` unit/integration tests (subject/sky/background/object/person + parts) |
| 20-in-30 workflow possible end-to-end | tooling shipped: keyboard culling, multi-select edit broadcast, copy/paste settings, background export queue; final timing validation tracked on issue #1 |
| `[HARD-LICENSE]` dependency licences | `cargo-deny` in CI with an explicit AGPL-compatible allowlist; any new crate whose licence is outside it fails the build rather than being reviewed after merge. Model weights are governed separately ([rnd/ml-models.md](rnd/ml-models.md)) — they are not dependencies of the AGPL work. |
| `[HARD-LOCAL]` no network in the app | Two layers: the same `cargo-deny` config bans networking crates from the application dependency graph (the fetch script is not part of it), and an integration test renders the fixture set with networking unavailable, asserting success. Catches the realistic failure — a transitive dependency phoning home — not just direct calls. |

Both rows are **specified, not yet wired** — they are the enforcement this
document previously left implicit. Until they land, each is a review-time
check; write them before adding the next dependency, not after.

## Determinism CI

- `focale-cli render <raw> <sidecar>` is the canonical export entry point.
- CI job matrix: `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64) render
  the committed fixture set and compare SHA-256 of output bytes; any divergence
  fails.
- Golden-file suites: (a) sidecar bytes for a canonical edit state, (b) frozen
  sidecars + frozen output hashes per pipeline version (regression), (c) colour
  transform vectors per gamut/format.
- macOS is a first-class target with **no CI** (`v1 (gap, issue #10)`,
  [platform](subsystems/platform.md)). Determinism CI is Linux-only on both
  architectures, so the cross-*architecture* claim is tested while the
  cross-*platform* claim is currently argued from the code's structure
  (pure-Rust libm, no platform branches on the export path) rather than
  demonstrated. Adding macOS to the render matrix is the check that would
  close it.

## Claims not currently enforced

Recorded so they are visible rather than assumed. Each is a known hole, not
an oversight:

- **Preview perceptual fidelity** (`[HARD-DET]`'s clause that the GPU path
  must be perceptually faithful to the CPU path). No automated CPU-vs-WGSL
  parity test exists; the operators are mirrored by hand and reviewed on
  change ([color](subsystems/color.md)). The clause is also unmeasurable as
  written — "perceptually faithful" has no threshold — so closing this
  means first choosing one (e.g. a bounded ΔE over a fixture set), then
  testing against it.
- **The < 100 ms preview budget** — a budget, never measured; instrumentation
  and a reproducible benchmark are `v1 (gap, issue #11)`
  ([preview](subsystems/preview.md)).
- **Export-recipe validity rules** ([sidecar](subsystems/sidecar.md) §5.16)
  are enforced by the encoder at runtime, but no test asserts that every
  invalid combination is rejected — a third-party writer's expectations rest
  on prose alone today.
