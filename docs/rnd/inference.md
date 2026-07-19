# R&D: Inference runtime selection

Which ML inference runtime Focale uses, for two distinct roles with opposite
requirements. Governing invariants: `[HARD-DET]`, `[HARD-LICENSE]`,
`[HARD-RUST]` ([invariants](../invariants.md)). Survey findings verified
2026-07-19 unless noted.

## The two roles and the split verdict

| Role | Determinism requirement | Verdict |
| --- | --- | --- |
| **1. Creation-time inference** — mask segmentation ([masks](../subsystems/masks.md)), the suggestion engine ([preview](../subsystems/preview.md)), future auto-culling assist. Results are resolved into the sidecar; the export path replays recorded data. | None (GPU permitted, tolerance-level variance fine). | **`ort` — decided, pinned.** |
| **2. Export-path neural stages** — neural denoise/sharpen (`v2 (committed)`), super-resolution (`high-priority future`). These run inside the versioned pipeline. | **Bit-identical f32 output across x86_64 and aarch64**, any thread count, forever (`[HARD-DET]`, `[HARD-VER]`). | **Open — no runtime qualifies today.** Candidate paths below; a stage ships only after the gate at the end of this doc passes. |

The split is itself the decision: committing one runtime to both roles would
either burden mask-time inference with determinism it doesn't need or ship
export-path stages on a runtime that cannot honor `[HARD-DET]`.

## Requirements

**Role 1:** ONNX ingestion of off-the-shelf models, maturity, GPU execution
providers available later, AGPL-compatible license, maintained upstream.

**Role 2 (all mandatory):**

- Bit-identical f32 across x86_64/aarch64 and across thread counts: no
  runtime-dispatched per-microarchitecture kernels, no fast-math, no
  compiler-contracted FMA, fixed reduction orders.
- Transcendentals through a pinned implementation (the `focale_core::math`
  /libm discipline — std float functions already bit us once,
  [verification](../verification.md)).
- Freezable per pipeline version: the executed kernel code must be pinnable
  forever ([pipeline](../subsystems/pipeline.md) versioning mechanics).
- AGPL-compatible; pure Rust strongly preferred (`[HARD-RUST]`, auditable
  freeze).

## Landscape (verified 2026-07-19)

| Runtime | License | Rust story | Cross-arch bit-determinism | ONNX | Notes |
| --- | --- | --- | --- | --- | --- |
| **ONNX Runtime** via `ort` | MIT (ORT); MIT/Apache-2.0 (`ort`) | `ort` 2.0.0-rc.12, wraps ORT 1.24; most-used Rust option | **No — explicitly disclaimed by maintainers**: "ORT never guarantees the output of a graph run will be deterministic" (issue #7642); different results across CPU architectures "expected" (#7642, #12086); "we can't promise it" even on CPU (#4611). `use_deterministic_compute` covers run-to-run kernel selection only. | Best-in-class | C++ dep, prebuilt binaries; the shipped v1 choice for role 1. |
| **tract** (Sonos) | MIT/Apache-2.0 | 0.23.4 (2026-07), very active, production at Sonos; no C++ toolchain | No claim; ruled out by construction: hand-written per-arch assembly FMA micro-kernels, runtime-dispatched; rayon-parallel matmul; own tests accept 1-ULP variance | Strong practical coverage (incl. `com.microsoft` contrib ops) | Lightest deployment of the capable engines; plausible `ort` alternative for role 1 if the C++ dependency ever hurts. Whether a generic non-SIMD kernel path can be pinned via public API: unverified. |
| **candle** (HF) | MIT/Apache-2.0 | candle-core 0.11.0, active | No claim; `gemm` crate does runtime microarch dispatch + rayon; optional MKL/Accelerate make results library-dependent | **Weak** — `candle-onnx` is an incomplete interpreter (`Resize` unsupported — disqualifying for image models) | Strength is native Rust ports, not ONNX ingestion. |
| **burn** (Tracel) | MIT/Apache-2.0 | 0.21.0; ndarray/CPU/no_std backends pure Rust | No cross-arch claim (its determinism PR #5156 targets GPU run-to-run only); `matrixmultiply` has per-arch FMA microkernels + threading — but backends are swappable | **`burn-onnx`: build-time codegen** — converts the ONNX graph to readable Rust source (~169/209 ops, opsets 1–24), weights loaded separately | The codegen property is the interesting one: generated Rust is auditable, freezable, and versionable — matching `[HARD-VER]` exactly. |
| **wonnx** | MIT/Apache-2.0 | — | N/A (WebGPU only) | Frozen at 2023 opsets | **Archived read-only 2025-05** — dead; not usable even for preview work. |
| **libtorch** via tch-rs | BSD-3 (PyTorch); MIT/Apache-2.0 (tch 0.24.0, libtorch 2.11) | Mature bindings | **No — explicitly disclaimed**: "not guaranteed across PyTorch releases … or different platforms"; different backend libraries per arch (MKL/oneDNN vs oneDNN+ACL, which may route f32 through bf16) | None (TorchScript/`torch.export` only) | Hundreds of MB of native library — disproportionate for a photo editor. |
| **ExecuTorch** | BSD-3; Rust crate `executorch` 0.9.0 (single maintainer, pins C++ 1.1.0, two minors behind) | Thin | **Plausible, unproven**: the Portable Kernel Library is plain reference C++17 with no SIMD dispatch and ahead-of-time memory planning — but zero published reproducibility evidence, and everything hinges on our own strict-FP compilation; the XNNPACK delegate forfeits it all | Not ONNX (`.pte` via `torch.export`) | The most promising *non-Rust* cross-arch story surveyed. |
| **LiteRT** (ex-TFLite) | Apache-2.0 | Bindings effectively stale (`tflite` dormant since 2024; `tflitec` wraps the old C API) | No — XNNPACK selects microkernels per microarchitecture at runtime; its own consistency flag is scoped to "the same compiled XNNPACK library" | None (`.tflite`; lossy community conversion) | Small binaries, wrong ecosystem fit. |

**The survey's central finding:** *no* mainstream runtime guarantees — or even
targets — bit-identical f32 CPU inference across architectures. Only two engage
determinism explicitly at all: ONNX Runtime (to disclaim it) and burn (GPU
run-to-run only). Export-path determinism must therefore come from
**Focale-controlled kernels**, not any runtime's defaults.

## Role 2 candidate paths

Two defensible architectures, to be prototyped when the first export-path
neural stage (denoise) is scheduled:

1. **`burn-onnx` codegen onto a pinned backend** *(leading candidate)*. The
   ONNX graph is compiled at build time into plain Rust source; we pin that
   generated source (checked in, reviewed, frozen per pipeline version) and run
   it on a deliberately scalar, fixed-iteration-order backend with
   transcendentals routed through `focale_core::math`. Pros: pure Rust
   (`[HARD-RUST]`), the executed code is exactly as auditable and freezable as
   our hand-written stages, weight files are hash-pinned like models today.
   Cons: default `matrixmultiply`/threading must be replaced or configured out;
   performance without SIMD needs measuring (export is allowed to be slow —
   "correct beats fast" — but not absurd).
2. **ExecuTorch portable kernels under strict-FP compilation** *(fallback)*.
   Reference C++ kernels, no runtime dispatch, compiled by us with no
   fast-math/FMA-contraction. Cons: C++ on the deterministic path, a thin
   single-maintainer Rust binding, `.pte` not ONNX, and the cross-arch claim is
   still only structural, not demonstrated.

Rejected for role 2 outright: `ort`, tch-rs/libtorch, LiteRT (maintainers or
structure explicitly rule out cross-arch bit-exactness); tract and candle (no
pinnable scalar path through the public API today — tract is worth re-checking,
it is the only mainstream engine where upstream might accept one).

## Decision gate (before any export-path neural stage ships)

1. Prototype the stage on the candidate path; freeze kernels + weights.
2. **Golden cross-arch validation**: render the fixture set on x86_64 and
   aarch64 CI and byte-diff outputs — the same harness as
   [verification](../verification.md) — across thread counts.
3. Benchmark on the preview base (the stage also runs in preview for
   super-resolution — [scope](../scope.md#high-priority-future)).
4. Record the decision here; the winning path graduates into the
   [pipeline](../subsystems/pipeline.md) doc as a versioned stage.
