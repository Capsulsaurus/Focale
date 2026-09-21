# Raw decode & demosaic

Pipeline stage 1 ([pipeline](pipeline.md)): raw file → linear camera RGB f32.
Owning code: `focale-core/src/decode`. Governing invariants: `[HARD-DET]`,
`[HARD-VER]`, `[HARD-RUST]` ([invariants](../invariants.md)).

- **Decode crate** (`v1 (shipped)`): `rawshift-image` with `arw` + `dng` features
  (the crate's two "stabilizing" formats). Supported today: Sony
  lossless-compressed ARW (compression type 7 — A7 IV / A7R V / A1 generation
  bodies) and DNG. Uncompressed (type 1) and lossy (type 8) ARW decode is
  tracked in rawshift as
  [#64](https://github.com/visualcommons/rawshift/issues/64); we surface a
  clear per-file error until it lands (`v1 (gap, issue #12)`). Initial camera
  support target: Sony ARW first-class; other makes as the decode crate
  allows.

  **Note on "upstream".** rawshift is a sibling project by the same author,
  not a third party, so the two gaps Focale waits on
  ([#63](https://github.com/visualcommons/rawshift/issues/63) optics metadata,
  [#64](https://github.com/visualcommons/rawshift/issues/64) compression) are
  scheduling questions rather than external dependencies. Both issues were
  filed 2026-08-15; before that, docs described these as blocked upstream
  while **nothing upstream tracked them** — the wait had no other end than
  someone writing it down.

  Also note the dependency is pinned to the published `rawshift-image` 0.1.1,
  while rawshift `master` has since split into per-format crates
  (`rawshift-image-arw` and siblings) over a `gamut-*` foundation. Adopting
  either fix means moving to that newer shape, which is more than a version
  bump.
- **Path:** `decode_raw()` (u16 CFA) → black-level subtract → demosaic →
  normalize `u16 / white_level` to **linear camera RGB f32**.
- **Demosaic is pinned per pipeline version** to `Bayer(Amaze)` — never `Auto`,
  which is content-adaptive and would break `[HARD-VER]`. rawshift offers AMaZE,
  RCD, and LMMSE for Bayer (plus Markesteijn for X-Trans); AMaZE is RawTherapee's
  Bayer default and the reference for low-ISO detail recovery, while RCD trades a
  little fine detail for fewer overshoot artifacts and speed — but speed is
  irrelevant on the export path ("correct beats fast") and preview runs on the
  downscaled base, so AMaZE's quality edge wins. LMMSE is superior on
  very-high-ISO frames; exposing the demosaic algorithm as a *recorded sidecar
  parameter* (never content-adaptive) would stay fully deterministic —
  `eventually` ([scope](../scope.md#eventually)). rawshift's row-parallel
  demosaic writes disjoint rows from immutable input, so it is deterministic
  under any thread count.
- **X-Trans is pinned to `Markesteijn`** per pipeline version, on the same
  terms as `Bayer(Amaze)` and for the same reason: the pin is a property of
  the *pipeline version*, not of which bodies happen to be supported, so it
  must be stated before a Fuji file is ever decoded rather than chosen ad hoc
  when one is. Markesteijn is the only X-Trans algorithm rawshift offers and
  the reference implementation elsewhere in the open-source ecosystem. No
  X-Trans body is exercised by the v1 fixture set, so the path is specified
  but untested — treat first Fuji support as requiring golden coverage, not
  merely a decode that runs.
- **Unsupported files fail per-file, never silently.** A file whose
  compression or sensor layout this build cannot decode surfaces a
  user-visible error naming the file and the specific unsupported property
  (e.g. "uncompressed ARW is not supported by this build"); the rest of the
  directory continues to load. Decode errors never fall back to a different
  algorithm or a partial image — a wrong-but-rendered frame is worse than a
  clear refusal.
- **Camera colour matrix** (XYZ→camera, DNG convention) comes from rawshift's
  bundled per-model database with dual-illuminant CCT interpolation; DNG files
  use their embedded `ColorMatrix1/2`. The `gamut` crate is not used at the
  decode stage (it is a codec library); it was evaluated and its current versions
  lack what export needs ([export](export.md)).
- **Optics metadata** parsed (or not) at decode time is the input to the optical
  corrections stage; the presence struct and its limits are specified in
  [optics](optics.md).

## Embedded previews (`v1 (shipped)`)

Owning code: `focale-core/src/decode/preview.rs`. **Off the deterministic
export path** — a preview is the camera's rendering, not Focale's, so it never
feeds an export.

Previews exist so a directory is browsable and cullable regardless of how far
raw decode support has got. A file whose raw payload Focale cannot develop
still shows, still rates, still flags.

Two conventions are read, because which one a file uses depends on who wrote
it:

- **TIFF/EP thumbnail tags** — `JPEGInterchangeFormat` / `-Length`
  (0x0201 / 0x0202).
- **Strip-based preview IFDs** — a full IFD with `Compression = 7` (JPEG)
  whose bitstream is located by `StripOffsets` / `StripByteCounts`
  (0x0111 / 0x0117). This is what modern DNG writers use, Apple ProRAW
  included.

The largest preview found wins. rawshift 0.1.1 reads only the first
convention, which is why every strip-based file reported "no thumbnail" while
carrying a multi-megapixel JPEG; its extractor is kept as a fallback for
formats whose previews live somewhere this reader does not look.

Previews are stored in sensor orientation, so IFD0's `Orientation` is returned
alongside and applied before display — without it, a portrait frame shows on
its side.

**Colour.** A preview carries its own ICC profile and must be converted, not
assumed. Apple writes Display P3; treating those bytes as sRGB shows every
thumbnail oversaturated and makes the filmstrip disagree with the viewport
rendering the same file. Conversion happens in `focale-app/src/thumbs.rs`
against the same display gamut the viewport shader uses. An untagged preview
is assumed sRGB; a profile that cannot be parsed falls back to sRGB rather
than dropping the image.

**Size.** An embedded preview can be full-resolution — 8064×6048 in the
corpus this was built against, ~145 MB of RGB once decoded — so thumbnail
decodes are bounded in flight and ordered nearest-first, and thumbnail
textures are evicted by distance from the open image.
