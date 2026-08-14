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
| **ONNX Runtime** via `ort` | MIT (ORT); MIT/Apache-2.0 (`ort`) | `ort` 2.0.0-rc.12 (2026-03-05), wraps ORT 1.24; upstream ORT is now 1.27.1 (2026-07-11), so `ort` is three minors behind and its api-17..24 multiversioning cannot link 1.25+ | **No — explicitly disclaimed by maintainers**: "ORT never guarantees the output of a graph run will be deterministic" (#7642); results on "CPUs with different arch" expected to differ (#7642); "even on CPU we don't expect it can produce identical result … we can't promise it" (#4611). `use_deterministic_compute` is **a no-op on the CPU EP** — its consumers are CUDA/WebGPU/training only. | Best-in-class | C++ dep, prebuilt binaries; the shipped v1 choice for role 1. |
| **tract** (Sonos) | MIT/Apache-2.0 | 0.23.4 (2026-07-08), very active, production at Sonos; no C++ toolchain | No claim; per-arch assembly FMA micro-kernels with runtime dispatch; tests accept 1-ULP variance. **Correction: rayon matmul is opt-in (`multithread-mm`) and off by default in `tract-core`**, and its matmul partitions M/N only — never K — so thread count does not change f32 results. | Strong practical coverage (incl. `com.microsoft` contrib ops) | See role-2 candidates: the pinnable-scalar-path question is now resolved. |
| **candle** (HF) | MIT/Apache-2.0 | candle-core 0.11.0 (2026-06-26), active | No claim; `gemm` crate does runtime microarch dispatch + rayon; optional MKL/Accelerate make results library-dependent | **Weak** — `candle-onnx` covers only ~82 ops. `Resize` has landed but is a nearest-neighbour shim that rejects ONNX's own default attributes; ConvTranspose, InstanceNorm/LayerNorm/GroupNorm, GridSample and TopK are absent — disqualifying for image models. | Strength is native Rust ports, not ONNX ingestion. |
| **burn** (Tracel) | MIT/Apache-2.0 | 0.21.0 (2026-05-07). **ONNX import has moved out to the separate `tracel-ai/burn-onnx` repo**; `burn-ndarray` is deprecated in favour of `burn-flex` (which uses `gemm` + SIMD + rayon by default — no better) | No cross-arch claim. Its determinism PR #5156 is **open, untriaged, one week old**, and scoped to GPU `atomicAdd` reductions only. **`matrixmultiply` cannot be configured scalar** — it has no `portable`/`no-simd` feature and `threading` is on by default via burn's own defaults; the scalar fallback kernel exists but nothing can select it. | **`burn-onnx`: build-time codegen** — converts the ONNX graph to readable Rust source (**exactly 169/209 ops**, opsets 1–24, but models must be upgraded to opset ≥16 before import), weights loaded separately | The codegen property is the interesting one: generated Rust is auditable, freezable, and versionable — matching `[HARD-VER]` exactly. |
| **wonnx** | MIT/Apache-2.0 | — | N/A (WebGPU only) | Frozen at 2023 opsets | **Archived read-only 2025-05** — dead; not usable even for preview work. |
| **libtorch** via tch-rs | BSD-3 (PyTorch); MIT/Apache-2.0 (tch 0.24.0, libtorch 2.11) | Mature bindings | **No — explicitly disclaimed**: "not guaranteed across PyTorch releases … or different platforms"; different backend libraries per arch (MKL/oneDNN vs oneDNN+ACL, which may route f32 through bf16) | None (TorchScript/`torch.export` only) | Hundreds of MB of native library — disproportionate for a photo editor. |
| **ExecuTorch** | BSD-3; Rust crate `executorch` 0.9.0 (single maintainer, pins C++ 1.1.0, two minors behind) | Thin | **Plausible, unproven**: the Portable Kernel Library is plain reference C++17 with no SIMD dispatch and ahead-of-time memory planning — but zero published reproducibility evidence, and everything hinges on our own strict-FP compilation; the XNNPACK delegate forfeits it all | Not ONNX (`.pte` via `torch.export`) | The most promising *non-Rust* cross-arch story surveyed. |
| **LiteRT** (ex-TFLite) | Apache-2.0 | Bindings effectively stale (`tflite` dormant since 2024; `tflitec` wraps the old C API) | No — XNNPACK selects microkernels per microarchitecture at runtime; its own consistency flag is scoped to "the same compiled XNNPACK library" | None (`.tflite`; lossy community conversion) | Small binaries, wrong ecosystem fit. |

**The survey's central finding:** *no* mainstream runtime guarantees — or even
targets — bit-identical f32 CPU inference across architectures. Only two engage
determinism explicitly at all: ONNX Runtime (to disclaim it) and burn (GPU
run-to-run only). Export-path determinism must therefore come from
**Focale-controlled kernels**, not any runtime's defaults.

Two external data points (verified 2026-07-20) make that finding stronger
rather than weaker, and are worth recording so this conclusion is not
re-litigated on a hunch:

- **A funded commercial team whose entire product is deterministic inference
  measured this exact question and published the result: 100% match on
  same-architecture runs, 0% cross-architecture** (EigenAI), attributing it to
  architectural differences in FMA and rounding. They ship custom kernels with
  fixed reduction ordering and *still* scope their guarantee to identical
  hardware SKUs. If the well-resourced specialist cannot offer cross-arch
  bit-exactness as a product, no general-purpose runtime is about to.
- **The research field has pivoted away from the problem.** The 2025–2026
  direction is *verifying results despite* nondeterminism rather than
  eliminating it. This implies no runtime will grow the feature we want, so
  waiting is not a strategy.

The corollary is that `[HARD-DET]`'s export path is genuinely differentiating:
no competing raw developer claims cross-machine bit-reproducibility, and
several ship GPU export paths that demonstrably preclude it
([verification](../verification.md) owns our own enforcement).

## Role 2 candidate paths

Four defensible architectures, to be prototyped when the first export-path
neural stage (denoise) is scheduled. Ordering revised 2026-07-20.

1. **`burn-onnx` codegen plus our own GEMM** *(leading candidate)*. The ONNX
   graph is compiled at build time into plain Rust source; we pin that
   generated source (checked in, reviewed, frozen per pipeline version) and run
   it on a deliberately scalar, fixed-iteration-order backend with
   transcendentals routed through `focale_core::math`. Pros: pure Rust
   (`[HARD-RUST]`), the executed code is exactly as auditable and freezable as
   our hand-written stages, weight files are hash-pinned like models today.
   **Cons, restated after verification: the backend cannot be configured, it
   must be written.** `matrixmultiply` exposes no scalar path and burn's
   replacement backend (`burn-flex`) is no better, so "configured out" was too
   soft — we would own the matmul. Also note ONNX import now lives in a
   separate repo and the canonical import documentation has moved; freezing
   generated code is practically easy (it is plain Rust under `OUT_DIR`) but is
   **not a documented upstream workflow**.
2. **tract with an upstream generic-kernel knob** *(newly promoted; may
   outrank 3)*. The previously-unverified question — whether a generic
   non-SIMD path can be pinned through the public API — is now **resolved:
   no such API, feature, or env var exists**. `OPS` is a `lazy_static`
   initialised from `best()` with no setter. But the delta is unusually small:
   `generic()` is *already public*, `best()` is literally `generic()` plus
   arch plugs, and rayon is already off by default — so a `TRACT_FORCE_GENERIC`
   knob is a small change against existing infrastructure, needing no new
   kernels. It must also cover the elementwise binary registries, which are
   separate `lazy_static`s. **No one has asked upstream.** Honest risk:
   upstream's instinct is to *relax* numerical assertions rather than defend
   them (PR #2406 responded to a 1-ULP divergence by widening the tolerance),
   so we would own cross-arch golden testing regardless — as we would under
   every candidate here.
3. **ExecuTorch portable kernels under strict-FP compilation** *(fallback)*.
   Reference C++17 kernels, no SIMD dispatch, compiled by us with no
   fast-math/FMA-contraction. **Build requirement, load-bearing: the same
   sources also compile into an `optimized_portable_kernels` target that pulls
   in ATen's SIMD `at::vec::Vectorized`. Must be built with
   `EXECUTORCH_BUILD_KERNELS_OPTIMIZED=OFF`** — getting this wrong silently
   forfeits the entire property. Cons: C++ on the deterministic path; the Rust
   binding is single-maintainer, unpublished for 5 months and pinning a C++
   release two versions behind; `.pte` not ONNX; and reproducibility evidence
   remains **empty** — upstream parity testing is tolerance-based
   (`atol=rtol=1e-3`), with zero hits for bitwise or cross-platform parity.
4. **Software floating point** *(the option that cannot fail on correctness)*.
   The only approach with an actual existence proof for cross-architecture
   bit-exactness is to stop using the hardware FPU — demonstrated across 8-bit
   AVR and 16-bit MSP430 in published work. Rust building blocks exist and are
   battle-tested: **`rustc_apfloat`** (the LLVM APFloat port rustc itself uses
   precisely so compile-time FP is deterministic), `softfloat`, `simple_soft_float`.
   Our export path is already CPU-only and explicitly allowed to be slow. If
   the gate's benchmark step is what kills candidates 1–3, this is the answer
   that fails only on speed — so it belongs in the gate from the start rather
   than being discovered after the others fall over.

Rejected for role 2 outright: `ort`, tch-rs/libtorch, LiteRT (maintainers or
structure explicitly rule out cross-arch bit-exactness); candle (op coverage
disqualifying for image models).

Two questions are **unverified and must be settled empirically in the gate's
first step**, under whichever candidate: whether rustc's `fp-contract` setting
lets LLVM contract `mul`+`add` into a per-target FMA inside scalar Rust
kernels, and whether polynomial transcendental approximations are cross-arch
bit-exact. The `focale_core::math`/libm discipline already anticipates the
second.

## Decision gate (before any export-path neural stage ships)

**The gate is a committed v2 deliverable in its own right**
([scope](../scope.md#v2-committed)), not a precondition someone will get to
when a feature is scheduled. The distinction matters because every neural
feature on the roadmap is blocked behind it: if the gate cannot be passed,
those features do not ship — there is no degraded-but-shipped fallback, since
a stage that cannot be bit-reproduced across architectures violates
`[HARD-DET]` at any quality level.

1. Prototype the stage on the candidate path; freeze kernels + weights.
2. **Golden cross-arch validation**: render the fixture set on x86_64 and
   aarch64 CI and byte-diff outputs — the same harness as
   [verification](../verification.md) — across thread counts.
3. Benchmark on the preview base. This is a pass/fail criterion, not a
   footnote: every pipeline stage runs in preview as well as export
   ([pipeline](../subsystems/pipeline.md)), so a kernel set that is
   deterministic but too slow to render interactively fails the gate as surely
   as a fast non-deterministic one.
4. Record the decision here; the winning path graduates into the
   [pipeline](../subsystems/pipeline.md) doc as a versioned stage.
