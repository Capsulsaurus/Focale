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

## Determinism CI

- `focale-cli render <raw> <sidecar>` is the canonical export entry point.
- CI job matrix: `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64) render
  the committed fixture set and compare SHA-256 of output bytes; any divergence
  fails.
- Golden-file suites: (a) sidecar bytes for a canonical edit state, (b) frozen
  sidecars + frozen output hashes per pipeline version (regression), (c) colour
  transform vectors per gamut/format.
