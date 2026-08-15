# Focale documentation

Focale is a deterministic, guided raw photo developer. This page is the entry
point to its specification: the subsystem map, the glossary, and the conventions
every doc follows. Start here; each topic has exactly one owning document.

## Doc tree

| Document | Owns |
| --- | --- |
| [invariants.md](invariants.md) | The six HARD principles and their permanent IDs. |
| [scope.md](scope.md) | Product definition; the status vocabulary; what ships in v1/v2/later/never. |
| [verification.md](verification.md) | How the docs' claims are enforced: tests, golden files, determinism CI. |
| [subsystems/pipeline.md](subsystems/pipeline.md) | Fixed stage order, working space, export-path determinism rules, versioning mechanics. |
| [subsystems/decode.md](subsystems/decode.md) | Raw decode, demosaic, camera colour matrices. |
| [subsystems/optics.md](subsystems/optics.md) | Optical corrections: source policy, parameter model, `CorrectionSource` seam. |
| [subsystems/masks.md](subsystems/masks.md) | Mask parity set, rasterization, AI segmentation stack. |
| [subsystems/color.md](subsystems/color.md) | Colour science, managed preview, tone/gamut mapping, wide-gamut capability matrix. |
| [subsystems/export.md](subsystems/export.md) | Output transform execution, codecs, HDR signaling. |
| [subsystems/preview.md](subsystems/preview.md) | Preview architecture, job scheduler, suggestion-engine stub contract. |
| [subsystems/app.md](subsystems/app.md) | Session model, editor, culling & XMP interop, batch, status bar. |
| [subsystems/platform.md](subsystems/platform.md) | Targets, GUI stack, performance targets, distribution & packaging. |
| [subsystems/sidecar.md](subsystems/sidecar.md) | The `.fcl` format: normative schema + deterministic encoding (stable §-numbers). |
| [rnd/lens-database.md](rnd/lens-database.md) | R&D: the open lens-correction profile database + measurement kit design. |
| [rnd/ml-models.md](rnd/ml-models.md) | R&D: ML model registry, distribution/licensing policy, model roadmap. |
| [rnd/inference.md](rnd/inference.md) | R&D: inference-runtime selection, incl. the deterministic export-path runtime. |

`subsystems/` is normative spec, kept in sync with the implementation in the
same PR as any behavior change. `rnd/` is design work for committed-but-unbuilt
areas; it becomes normative by graduating into a subsystem doc.

## Subsystem ↔ crate map

| Crate | Role | Spec |
| --- | --- | --- |
| `focale-core` | Everything on the deterministic path: decode wrapper, pipeline stages, colour math, mask rasterization, retouch, geometry. CPU-only, no GUI deps, no build script. | [pipeline](subsystems/pipeline.md), [decode](subsystems/decode.md), [optics](subsystems/optics.md), [masks](subsystems/masks.md), [color](subsystems/color.md) |
| `focale-sidecar` | The `.fcl` sidecar: schema types, deterministic CBOR writer. No build script; writers pass provenance strings in. | [sidecar](subsystems/sidecar.md) |
| `focale-export` | Output transform and export encoders. | [export](subsystems/export.md) |
| `focale-segment` | ONNX segmentation (ort). Used only at mask-creation time; never on the export path. | [masks](subsystems/masks.md), [rnd/inference](rnd/inference.md) |
| `focale-buildinfo` | Build provenance strings (release version + git short hash, platform name) for the writing binaries; keeps the deterministic-path crates free of build scripts. | [sidecar](subsystems/sidecar.md) §5.1 |
| `focale-cli` | Headless export binary. The reference deterministic path; CI runs it on x86_64 + aarch64 and diffs bytes. Also hosts `bench-preview`, the offline preview-latency benchmark. | [verification](verification.md) |
| `focale-app` | Desktop GUI (eframe = winit + wgpu + egui). Depends on core/sidecar/segment/export/buildinfo. | [app](subsystems/app.md), [preview](subsystems/preview.md), [platform](subsystems/platform.md) |

**Rationale for the split:** the export path must be testable headless on CI
across architectures, so nothing in `focale-core` may depend on GUI or GPU
crates. `focale-segment` is isolated because model inference is explicitly *not*
part of the deterministic path (masks are resolved into the sidecar at creation
time, [masks](subsystems/masks.md)).

## Glossary

| Term | Meaning | Owning doc |
| --- | --- | --- |
| **HARD principle** | A non-negotiable requirement with a permanent citable ID (`[HARD-DET]`…). | [invariants](invariants.md) |
| **Export path** | The CPU-only code path from raw+sidecar to output bytes; must be bit-identical everywhere. | [pipeline](subsystems/pipeline.md) |
| **Pipeline version** | Frozen algorithm set a sidecar was edited with; renders identically forever. | [pipeline](subsystems/pipeline.md), mechanics in [sidecar](subsystems/sidecar.md) §3 |
| **Working space** | Linear Rec.2020 primaries, f32, unbounded, until the output transform. | [pipeline](subsystems/pipeline.md) |
| **Sidecar / `.fcl`** | Per-image CBOR document holding the complete edit; the raw is never modified. | [sidecar](subsystems/sidecar.md) |
| **CDE** | Core Deterministic Encoding (RFC 8949 §4.2): identical documents → identical bytes. | [sidecar](subsystems/sidecar.md) §2 |
| **LiveIndex** | Sidecar block the directory view is rebuilt from by scanning (`[HARD-FS]`). | [sidecar](subsystems/sidecar.md) §5.13 |
| **Culling mirror** | Derived one-way XMP sidecar for rating/label interop; never read back. | [app](subsystems/app.md) |
| **Seam** | A deliberately kept integration point for a committed future feature. | [scope](scope.md) (each seam in its subsystem doc) |
| **Resolved mask** | AI segmentation output baked to a bitmap at creation time; models never run at export. | [masks](subsystems/masks.md) |
| **`CorrectionSource`** | The trait seam feeding optical-correction parameters from any source. | [optics](subsystems/optics.md) |
| **Preview base** | ≤ 2560 px mip level every interactive edit re-renders on the CPU; also the resolution AI masks resolve to. | [preview](subsystems/preview.md) |
| **Active rendering gamut** | User-selected display gamut, always visible in the status bar. | [color](subsystems/color.md) |
| **Export recipe** | Named, byte-complete export configuration stored in the sidecar. | [sidecar](subsystems/sidecar.md) §5.14, executed per [export](subsystems/export.md) |
| **Golden fixture** | Committed byte-exact artifact CI diffs against; re-blessed only deliberately. | [verification](verification.md) |
| **Model manifest** | Pinned registry (hash, license, source) every ML model is fetched through. | [rnd/ml-models.md](rnd/ml-models.md) |

## Doc conventions

- **Single source of truth.** Every fact, decision, and definition has exactly
  one owning document; every other mention links to it. Restating is a bug —
  fix by replacing the copy with a link.
- **Status tags.** Every feature carries one tag from the vocabulary owned by
  [scope.md](scope.md): `v1 (shipped)` / `v1 (gap, issue #N)` / `v2 (committed)`
  / `high-priority future` / `eventually` / `demand-driven` / `never`.
- **Decision records.** Non-obvious decisions state: what was decided, the
  deciding factors, alternatives considered with rejection reasons, and any
  load-bearing external finding with its verified-on date.
- **Citations from code.** Code comments cite HARD IDs and doc *file paths*
  (e.g. `docs/subsystems/masks.md`) — never section numbers, which rot under
  editing. Sole exception: [subsystems/sidecar.md](subsystems/sidecar.md)
  declares its §-numbers stable and may be cited by section.
- **Sync rule.** A PR that changes documented behavior updates the owning doc in
  the same PR. Docs note the issue number wherever a described seam is not yet
  implemented.
