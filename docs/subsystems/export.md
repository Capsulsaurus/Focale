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
  bytes** — are specified in [sidecar](sidecar.md) §5.14.

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

## Watchlist (mid-2026)

`jxl-oxide` remains decode-only; Imazen's pure-Rust `jxl-encoder` 0.3.x
(AGPL-3.0-only OR commercial) is pre-1.0 with unverified HDR signaling — watch,
don't adopt; `ravif` wraps the same rav1e pair but hides the threading/CICP knobs
determinism requires — direct use is correct; `gamut` 0.3 is still 8-bit SDR
TIFF/AVIF/WebP only (no JXL/PNG/JPEG/16-bit/HDR/ICC), so it cannot serve export;
revisit as it matures.

## Gain-map export

`v2 (committed)` ([scope](../scope.md#v2-committed)): the seam is kept in the
export-recipe schema (a recipe carries an optional `gain_map` block, rejected at
execution in v1 — [sidecar](sidecar.md) §5.14).
