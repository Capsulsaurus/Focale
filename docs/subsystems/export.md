# Export

Pipeline stage 11 execution ([pipeline](pipeline.md)): output transform and
encoders. Owning code: `focale-export`; the CLI (`focale-cli`) is the reference
deterministic entry point ([verification](../verification.md)). Governing
invariants: `[HARD-DET]`, `[HARD-VER]`, `[HARD-LICENSE]`
([invariants](../invariants.md)).

- **HARD:** support SDR and HDR output. HDR uses the full capability of each
  format (PQ/HLG transfer, wide gamut, gain maps where the format supports
  them). The wide-gamut working space is mapped into whatever target the user
  selects ([color](color.md)).
- Export recipes — the schema recording **every option that affects output
  bytes** — are specified in [sidecar](sidecar.md) §5.14, and which recipes
  are *valid* in §5.16. Invalid combinations are rejected with a per-recipe
  error, never silently substituted.

## Colour signalling

Each codec carries the output gamut in whatever its container natively
supports; the goal is that no file is ever labelled with a space it is not in.

- **ICC-carrying formats** (TIFF, PNG SDR, JPEG) embed a profile generated
  in-crate by `focale-export::icc` — a minimal ICC v4 matrix/TRC profile built
  from the same `focale_core::color` primaries the pipeline used, with zeroed
  timestamps/IDs so the bytes are reproducible (`[HARD-DET]`). Generating
  rather than shipping vendor profiles keeps the file's label and the maths
  provably identical, and avoids bundling profiles of uncertain licence.
- **CICP-signalling formats** (AVIF always, PNG for HDR-PQ) carry an H.273
  code-point triple instead. This is what makes Adobe RGB unrepresentable in
  AVIF (§5.16): H.273 has no code point for its primaries.
- **JPEG XL** uses its own colour-encoding header rather than ICC.

## Codecs (`v1 (shipped)`)

All licenses verified AGPL-compatible, `[HARD-LICENSE]`; encoders run
single-threaded with pinned settings so output bytes are reproducible,
`[HARD-DET]`:

- TIFF 16-bit: `tiff` (MIT) — the designated hand-off format, with embedded ICC.
- PNG: `png` (MIT/Apache) — 16-bit, cICP chunk for HDR (PQ) + ICC for SDR.
- JPEG: `jpeg-encoder` (MIT/Apache) — 8-bit + ICC.
- JPEG XL: `jpegxl-rs` (GPL-3.0-or-later bindings over BSD-3 libjxl) — 16-bit,
  lossless option, HDR (PQ/HLG). License compatibility: **AGPLv3 §13** grants
  the AGPL-covered work (Focale) permission to link/combine with GPLv3-licensed
  works and convey the result, with the GPLv3 part remaining under GPLv3; GPLv3
  §13 is the mirror permission on the GPL side.
- AVIF: `rav1e` (BSD-2) + `avif-serialize` (BSD-3) — 8/10/12-bit per the recipe,
  CICP-signaled PQ/HLG and wide gamut. (Adobe RGB has no H.273 code point and is
  rejected for AVIF.)

## Watchlist (verified mid-2026)

Crates evaluated and *not* adopted, recorded so the evaluation is not repeated:

| Crate | Status | Why not (yet) |
| --- | --- | --- |
| `jxl-oxide` | Decode-only | Cannot encode; no path to replacing `jpegxl-rs`. |
| `jxl-encoder` (Imazen) 0.3.x | Watch, don't adopt | Pure Rust and AGPL-3.0-only OR commercial (compatible), but pre-1.0 with unverified HDR signalling. Attractive if it matures — it would remove the C++ libjxl dependency. |
| `ravif` | Rejected | Wraps the same rav1e + avif-serialize pair we use, but hides the threading and CICP knobs `[HARD-DET]` requires. Using the pair directly is correct. |
| `gamut` 0.3 | Rejected | 8-bit SDR TIFF/AVIF/WebP only — no JXL/PNG/JPEG, no 16-bit, no HDR, no ICC. Cannot serve export at any bit depth we ship. |

## Gain-map export

`v2 (committed)` ([scope](../scope.md#v2-committed)): the seam is kept in the
export-recipe schema (a recipe carries an optional `gain_map` block, rejected at
execution in v1 — [sidecar](sidecar.md) §5.14).

Gain maps are a *format* capability, not a universal one: they are defined for
JPEG (ISO 21496-1 / Adobe's HDR gain map), **HEIF/HEIC** (where the `'tmap'`
derived-item mechanism originates), AVIF, and JPEG XL, and have no meaning for
TIFF or PNG, which signal HDR through transfer characteristics alone. This is
why HDR JPEG is rejected in v1 rather than merely unimplemented (§5.16):
baseline JPEG has no other way to carry HDR, so the format's HDR support and
its gain-map support are the same feature.

**Standard status (verified 2026-07-20):** **ISO 21496-1:2025 is published**
(edition 1, 2025-07-07, ISO/TC 42) — the hedging in earlier drafts of this doc
can be dropped. Google's Ultra HDR v1.1 (2024-10-25) states that Ultra HDR and
ISO 21496-1 metadata "can coexist in a single JPEG file… During encoding,
implementations should encode both. During decoding, prefer ISO 21496-1
metadata when both are present" — so dual-encoding is the specified behaviour,
not a hedge. One caveat to record: the AVIF v1.2.0 specification never cites
ISO 21496-1; it only permits HEIF's `'tmap'`. The binding of ISO metadata into
`'tmap'` is established by *libavif's implementation*, not by AVIF spec text.

**Format choice for the v2 seam: JPEG Ultra HDR. Scope AVIF gain-map output
out of v1 and v2.** This is forced by `[HARD-RUST]`, not preference: a pure-Rust
gain-map path exists for JPEG (`ultrahdr-core`, Apache-2.0, `no_std + alloc`,
`#![forbid(unsafe_code)]`, writes both ISO 21496-1 and Adobe XMP, and takes
*pre-encoded* JPEGs so our own encoder stays in control), whereas **AVIF gain
maps have no Rust path at all** — `ravif`/`rav1e`/`avif-serialize` carry no
`tmap`/`altr` support, and `libavif-sys` binds libavif 1.0.4, which predates
gain-map support entirely.

> **`[HARD-LICENSE]` blocker — needs a human legal read before any gain-map
> work starts.** libultrahdr ships an `adobe-hdr-gain-map-license/NOTICE`
> reading "This product includes Gain Map technology under license by Adobe."
> That is a **patent grant to "this product"** — not an OSI licence, not
> obviously transferable, and **not avoided by writing our own encoder**,
> since it attaches to the technology rather than to any particular
> implementation. Its interaction with AGPL-3.0 §11 is exactly the kind of
> question `[HARD-LICENSE]` exists to force before adoption, not after. This
> is currently the single largest unaddressed risk in the v2 gain-map plan.

Two `[HARD-DET]` cautions for when it does proceed: `ultrahdr-core`'s SIMD
paths are of unverified bit-exactness (prefer owning the gain-map maths and
using the crate for metadata/container assembly only), and the encode is
`pow`/`exp2`-heavy so it needs the same `focale_core::math` pinning as the rest
of the export path, with `floor(recovery * 255.0 + 0.5)` quantization at a
fixed evaluation order. Encode is per-pixel with no reduction hazards.
