# The `.fcl` Sidecar Format — Schema v1

This document specifies the Focale sidecar file format precisely enough to
implement an independent reader or writer. It is the published schema
required by architecture.md §7. The reference implementation is
`crates/focale-sidecar` (`schema.rs` for the document tree, `cde.rs` for the
encoder); the byte-level contract is frozen by the golden test fixture
`crates/focale-sidecar/tests/golden/canonical.fcl`.

## 1. Overview

- One sidecar per image, stored **alongside the image**. The raw file is
  never modified.
- File name: the full image file name with `.fcl` appended —
  `IMG_0001.ARW` → `IMG_0001.ARW.fcl`. The extension is the only naming
  convention; file names and directory shape carry no other meaning.
- Content: a single CBOR (RFC 8949) document containing the schema and
  pipeline versions, the full parameter set for every pipeline stage
  (including resolved masks and retouch strokes), a live-index metadata
  block, and export recipes.
- A sidecar plus the raw file plus the recorded pipeline version is
  sufficient to reproduce an export **bit-identically, forever**.
- The document also carries two debug-provenance fields
  (`focale_version`, `focale_platform`, §5.1) recording which build last
  wrote it. They are informational only and never influence rendering.

## 2. Encoding

### 2.1 Envelope

The document is wrapped in CBOR tag **55799** ("self-described CBOR",
RFC 8949 §3.4.6), so every `.fcl` file begins with the bytes
`d9 d9 f7`, followed by the top-level map. Writers MUST emit the tag;
readers MUST also accept an untagged document.

### 2.2 Deterministic encoding (writers)

Writers MUST produce RFC 8949 **§4.2 Core Deterministic Encoding**:

- **Definite lengths only.** No indefinite-length strings, arrays, or maps.
- **Shortest-form integers and lengths.** The argument of every head uses
  the smallest width that can hold it (0–23 inline, then 1/2/4/8 bytes).
- **Shortest-form floats.** Each float is encoded in the shortest of
  float16 / float32 / float64 that round-trips the value exactly.
  `NaN` is canonicalized to `f9 7e 00`. (Focale never writes NaN, but the
  rule holds if a value tree contains one.)
- **Map keys sorted** by bytewise lexicographic order of their *encoded*
  key bytes. Since all schema keys are text strings, this is: shorter keys
  first, then bytewise UTF-8 order.
- **No duplicate keys** (writers reject them; a document with duplicates
  is malformed).

Identical documents therefore always serialize to identical bytes — this
is what makes sidecar bytes hashable and diffable in CI.

Note the carve-out: identical *edits* on different machines or builds do
**not** imply identical sidecar bytes, because the debug-provenance
fields `focale_version` / `focale_platform` (§5.1) vary by writer.
Byte-equality claims apply to identical *documents*; any edit-equality
comparison must ignore the two provenance fields.

### 2.3 Reading (readers)

Readers MUST accept any *well-formed* CBOR encoding of the document
(indefinite lengths, non-shortest forms, unsorted keys), not just CDE —
only Focale's writer output is guaranteed canonical. Readers MUST:

- ignore unknown map keys anywhere in the tree (forward tolerance),
- treat a missing field as its documented default (§5 tables),
- reject a document whose `schema_version` is greater than the newest
  schema they implement (see §3).

### 2.4 Type mapping conventions

The tree below is described in Rust-ish notation; it maps to CBOR as
follows:

| Schema construct | CBOR encoding |
| --- | --- |
| struct | map; keys are the field names as text strings |
| `bool` | `false` / `true` (simple values 20/21) |
| `u8`/`u32`/`u64` | unsigned integer (major type 0) |
| `f32` | float (major type 7, shortest round-tripping width) |
| `String` | text string (major type 3) |
| `Vec<T>`, `[T; N]` | array (major type 4) |
| `[f32; 2]` point | 2-element array `[x, y]` |
| `Option<T>` | `null` (simple value 22) or the encoding of `T`; fields are always written, including `null`s |
| byte fields (`deflate_bitmap`, `thumbnail_hash`) | byte string (major type 2) — never an array of integers |
| enum, unit variant | the variant name as a text string, e.g. `"Pick"` |
| enum, variant with fields | one-entry map `{ "VariantName": payload }`, where the payload of a struct variant is a map of its fields |

Variant and field names are case-sensitive and match the Rust identifiers
exactly (e.g. `"AsShot"`, `"JpegXl"`, `"chromatic_aberration"`).

## 3. Versioning policy

### 3.1 The two versions

Two independent version numbers appear in every document:

- **`schema_version`** — the version of *this file format*. Current: **1**.
- **`pipeline_version`** — the version of the *processing algorithms* the
  edit was made with (`focale_core::PIPELINE_VERSION` at creation, or the
  version the user last explicitly upgraded the document to, §3.3).
  Exports must render with that version's algorithms forever.

### 3.2 How pipeline permanence is enforced

The guarantee "old sidecars render identically forever" is not a
convention readers are asked to follow — it is mechanized in the
reference implementation, and third-party implementations should mirror
the same structure:

- **A single dispatch point.** `focale_core::pipeline::render(input,
  version)` is the only place a pipeline version is ever selected. It
  matches on the number and fails with an "unsupported pipeline version"
  error for any version the build does not implement — future versions
  are never guessed at, mirroring the future-schema rule (§3.4).
- **Frozen per-version module trees of pipe-filter stages.** Each
  version is a module tree (`pipeline::v1`, …) of pure stage functions —
  `v1::tone::apply(image, &params)`, `v1::geometry::apply(…)`, and so on
  for every stage. Each stage is an encapsulated filter over
  `(image, params)` with no shared pipeline state; constants, iteration
  orders, kernels, and interpolation are pinned. Once a version ships,
  output-changing edits to its tree are forbidden — even bug fixes: a
  fix that changes output becomes part of the *next* version.
- **How a new version is added while old ones stay re-runnable.** A `v2`
  gets its own `render` entry and new stage modules **only for the
  stages whose output changes**; for every unchanged stage, `v2::render`
  calls the existing `v1` stage function directly. Because stages are
  pure functions, this reuse cannot drift. Even when v2 changes a
  stage's defaults or semantics, a document stamped `pipeline_version: 1`
  still flows through `v1::render` via the dispatcher — every old
  stage remains re-runnable exactly as shipped, regardless of what the
  new version's defaults are. The dispatcher gains exactly one arm per
  version; nothing else changes.
- **Rendering always uses the stored version.** The GUI (preview, edit,
  export) and the CLI both dispatch on the document's `pipeline_version`
  — never silently on the build's current version.

### 3.3 Older versions in the UI: warning and explicit upgrade

- Opening a document whose `pipeline_version` is older than the build's
  current version produces a persistent status-bar warning
  ("edited with older pipeline vN"), emitted by the render dispatcher as
  a render warning. Rendering continues with vN's algorithms.
- The status bar offers an explicit **"Upgrade to v{current}"** action
  that re-stamps `pipeline_version`; the edit is then reinterpreted
  under the current algorithms. This is the **only** operation that ever
  changes a document's `pipeline_version` — editing never re-stamps.
- A `pipeline_version` *newer* than the build (possible within an
  accepted `schema_version`) is not a load error; rendering fails with a
  clear "unsupported pipeline version" message instead of guessing.

### 3.4 Schema evolution rules

Rules (permanent compatibility — no exceptions, no deprecation):

- **Newer software reads every older schema forever.** Schema readers keep
  support for all versions `1..=current`.
- **Additive changes within a schema version** are allowed: new map keys
  may appear with documented defaults. Old readers ignore them; new
  readers default them when absent. Additions must never change the
  meaning of existing fields.
- **Renaming, removing, or re-typing a field** requires bumping
  `schema_version`; the old form must remain readable forever.
- **Future versions are rejected**, not guessed at: a reader encountering
  `schema_version > current` fails with a "future schema" error rather
  than silently dropping data it does not understand.
- Any intentional change to the encoded bytes of the canonical document is
  a schema change and must be accompanied by a golden-fixture re-bless
  (`FOCALE_BLESS=1`) in the same change.

## 4. Coordinate and unit conventions

- Mask and retouch coordinates are normalized to the **pre-geometry**
  working frame: `x, y ∈ [0, 1]`, y down. Masks and retouch therefore
  survive crop changes without re-anchoring.
- Lengths described as "fraction of the long image edge" are also
  normalized to that same frame.
- Slider-style parameters use the conventional −100..=+100 (or 0..=100)
  raw-developer scales; exact ranges are noted per field. Ranges are
  clamped at the UI boundary, **not** by readers — the pipeline renders
  whatever the sidecar says, forever.
- Curve and luminance values on "the display-referred [0,1] axis" refer to
  the tone-curve domain used by the pipeline.

## 5. Document tree

Top-level value: tag 55799 around a map — `SidecarDoc`.

### 5.1 `SidecarDoc`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `schema_version` | u32 | 1 | Schema version that wrote the file. |
| `pipeline_version` | u32 | 1 | Pipeline version of the edit (see §3). |
| `edit` | `EditState` | all-default | Full parameter set for every stage (§5.2). |
| `live_index` | `LiveIndex` | all-default | Directory-view index block (§5.13). |
| `export_recipes` | array of `ExportRecipe` | `[]` | Named export configurations (§5.14). |
| `focale_version` | String or null | null | Debug provenance: the Focale build that last wrote this file, `"<release>+<short-git-hash>"` (e.g. `"0.1.0+e258182"`; the hash segment is `"unknown"` when built outside git). null = written by a pre-provenance build. |
| `focale_platform` | String or null | null | Debug provenance: OS the writer ran on — `"linux"`, `"macos"`, or `"windows"` (Rust `std::env::consts::OS` names). Same rules as `focale_version`. |

The two `focale_*` fields exist solely for debugging field reports. The
writing application re-stamps both on **every save**; readers MUST NOT
branch on them, and they are excluded from all determinism/equality
claims (§2.2 carve-out).

### 5.2 `EditState`

Field order below mirrors the fixed pipeline stage order (architecture.md §3). Raw
decode (stage 1) has no user parameters; the output transform (stage 11)
is parameterized by the export recipe.

| Key | Type | Stage |
| --- | --- | --- |
| `optics` | `OpticsParams` | 2 — optical corrections |
| `white_balance` | `WhiteBalanceParams` | 3 — white balance |
| `tone` | `ToneParams` | 4 — global tone |
| `color` | `ColorParams` | 5 — global colour |
| `local` | array of `LocalAdjustment` | 6 — local adjustments |
| `detail` | `DetailParams` | 7 — detail |
| `retouch` | `RetouchParams` | 8 — retouch |
| `geometry` | `GeometryParams` | 9 — geometry |
| `finishing` | `FinishingParams` | 10 — finishing |

### 5.3 `OpticsParams` (stage 2)

Corrections come exclusively from embedded raw metadata in v1; toggles
have no effect when the corresponding metadata is missing.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable for the stage. |
| `vignetting` | bool | `true` | Correct vignetting. |
| `chromatic_aberration` | bool | `true` | Correct lateral CA. |
| `distortion` | bool | `true` | Correct geometric distortion. |

### 5.4 `WhiteBalanceParams` (stage 3)

An enum. Default: `"AsShot"`.

- `"AsShot"` (text) — the camera's as-shot multipliers from raw metadata.
- `{ "Temperature": { "kelvin": f32, "tint": f32 } }` — correlated colour
  temperature in kelvin (typ. 2000–50000) and green–magenta tint
  (0 = neutral, negative = green, positive = magenta).
- `{ "Custom": { "red": f32, "blue": f32 } }` — raw channel multipliers
  relative to green (g = 1), as sampled from a neutral patch.

### 5.5 `ToneParams` (stage 4)

| Key | Type | Default | Range / meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable. |
| `exposure` | f32 | 0 | EV stops, −5..=+5. |
| `contrast` | f32 | 0 | −100..=+100, around middle grey. |
| `highlights` | f32 | 0 | −100..=+100. |
| `shadows` | f32 | 0 | −100..=+100. |
| `whites` | f32 | 0 | −100..=+100. |
| `blacks` | f32 | 0 | −100..=+100. |
| `curve` | `ToneCurve` | identity | Point curve after the parametric controls. |

`ToneCurve` — `{ "points": [CurvePoint, …] }`. Points are kept sorted by
`x` and interpolated with a monotone cubic (Fritsch–Carlson). The identity
curve is exactly two points, (0,0) and (1,1).
`CurvePoint` — `{ "x": f32 ∈ [0,1], "y": f32 ∈ [0,1] }`.

### 5.6 `ColorParams` (stage 5)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable. |
| `hsl` | `HslBands` | all 0 | Per-band HSL (below). |
| `grading` | `ColorGrading` | all 0 | Three-way grading wheels (below). |
| `vibrance` | f32 | 0 | −100..=+100, weighted toward muted colours. |
| `saturation` | f32 | 0 | −100..=+100, uniform. |

`HslBands` — three arrays of exactly 8 f32 (−100..=+100 each, 0 =
neutral): `hue`, `saturation`, `luminance`. Band order is fixed:
red, orange, yellow, green, aqua, blue, purple, magenta.

`ColorGrading`:

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `shadows` / `midtones` / `highlights` | `GradingWheel` | all 0 | Wheel per zone. |
| `blending` | f32 | 0 | 0..=100, blending between zones. |
| `balance` | f32 | 0 | −100..=+100, shadows↔highlights balance. |

`GradingWheel` — `{ "hue": f32 ∈ [0,360), "saturation": f32 0..=100 (0 =
no tint), "luminance": f32 −100..=+100 }`.

### 5.7 `LocalAdjustment` (stage 6)

| Key | Type | Meaning |
| --- | --- | --- |
| `enabled` | bool | Whether this adjustment is active. |
| `mask` | `MaskGroup` | Where the adjustment applies (§5.8). |
| `adjustments` | `LocalParams` | The parameter deltas. |

`LocalParams` — every value is a delta/offset with 0 = no change
(default), so an empty adjustment is a no-op:

| Key | Type | Range / meaning |
| --- | --- | --- |
| `exposure` | f32 | EV offset. |
| `contrast`, `highlights`, `shadows`, `whites`, `blacks` | f32 | −100..=+100 offsets. |
| `curve` | `ToneCurve` | Point curve within the mask (default identity). |
| `temperature` | f32 | −100..=+100, mired-scaled WB offset. |
| `tint` | f32 | −100..=+100. |
| `tint_wheel` | `GradingWheel` | Single grading tint within the mask. |
| `vibrance`, `saturation` | f32 | −100..=+100 offsets. |

### 5.8 Masks

`MaskGroup` — `{ "name": String, "components": [MaskComponent, …] }`.
Components combine **in array order**; the first component's op applies
against an empty (all-zero) coverage.

`MaskComponent`:

| Key | Type | Range / meaning |
| --- | --- | --- |
| `op` | `MaskOp` | `"Add"` (`max(acc, m)`), `"Subtract"` (`acc·(1−m)`), or `"Intersect"` (`acc·m`). |
| `invert` | bool | Invert this component's own coverage before combining. |
| `feather` | f32 | Edge feather radius as a fraction of the long edge, 0..=0.25. |
| `density` | f32 | Maximum-opacity scale, 0..=1. |
| `shape` | `MaskShape` | The shape (one-entry map, below). |

`MaskShape` variants:

- `{ "Brush": { "strokes": [BrushStroke, …] } }` — strokes apply in paint
  order. `BrushStroke`: `erase` (bool — subtracts instead of paints),
  `radius` (f32, fraction of long edge), `feather` (f32 0..=1, fraction of
  radius; 0 hard, 1 fully soft), `flow` (f32 (0,1], per-stamp opacity
  accumulation), `points` (array of `[x, y]` stamp centres).
- `{ "Linear": { "start": [x,y], "end": [x,y] } }` — coverage 1 at/behind
  `start`, falling to 0 at `end`.
- `{ "Radial": { "center": [x,y], "radius": [rx,ry], "rotation": f32,
  "falloff": f32 } }` — coverage 1 inside the inner ellipse, 0 at the
  outer boundary; `rotation` in degrees CCW; `falloff` ∈ [0,1] is the
  fraction of the radius over which coverage falls 1→0.
- `{ "LuminanceRange": { "low": f32, "high": f32, "falloff": f32 } }` —
  band over working-space luminance Y (Rec.2020 weights) on the
  display-referred [0,1] axis; `falloff` ∈ [0,1] softens the band edges.
- `{ "ColorRange": { "color": [r,g,b], "tolerance": f32, "falloff": f32 } }`
  — coverage from distance to a sampled colour; `color` is linear
  Rec.2020; `tolerance` is the acceptance radius in Oklab distance
  (0..=1); `falloff` ∈ [0,1].
- `{ "AiResolved": ResolvedMask }` — AI segmentation output **resolved at
  creation time**; a model never runs on the export path.

`ResolvedMask`:

| Key | Type | Meaning |
| --- | --- | --- |
| `kind` | `SegmentKind` | What was segmented (below). |
| `width`, `height` | u32 | Bitmap dimensions in pixels (typically 1/2 raw resolution). |
| `deflate_bitmap` | bytes | Deflate(zlib)-compressed 8-bit coverage, row-major, exactly `width × height` bytes when decompressed; 255 = full coverage. Upsampled bilinearly at render time. |

`SegmentKind` variants: `"Subject"`, `"Sky"`, `"Background"`, `"Object"`,
`{ "Person": { "index": u8 } }` (0-based, distinguishing multiple
people), `{ "PersonPart": { "index": u8, "part": PersonPart } }`.
`PersonPart` variants: `"FaceSkin"`, `"BodySkin"`, `"Hair"`,
`"Eyebrows"`, `"Sclera"`, `"Iris"`, `"Lips"`, `"Teeth"`, `"Clothing"`.

### 5.9 `DetailParams` (stage 7)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable. |
| `sharpen` | `SharpenParams` | see below | Capture sharpening. |
| `noise_reduction` | `NoiseReductionParams` | all 0 | Conventional NR. |

`SharpenParams` — `method` (`"Unsharp"` or `"Deconvolution"`, default
`"Unsharp"`), `amount` (f32 0..=150, default 40), `radius` (f32 0.5..=3.0
pixels, default 1.0), `masking` (f32 0..=100, default 0; higher restricts
sharpening to stronger edges).

`NoiseReductionParams` — `luminance`, `luminance_detail`, `chroma`,
`chroma_detail`, each f32 0..=100, default 0.

### 5.10 `RetouchParams` (stage 8)

`{ "enabled": bool (default true), "strokes": [RetouchStroke, …] }`,
strokes applied in order. `RetouchStroke`:

| Key | Type | Meaning |
| --- | --- | --- |
| `mode` | text | `"Clone"` (copy source verbatim) or `"Heal"` (match destination at the boundary). |
| `radius` | f32 | Stamp radius, fraction of the long edge. |
| `feather` | f32 | 0..=1, fraction of radius. |
| `opacity` | f32 | (0,1]. |
| `dest` | array of `[x,y]` | Destination path; a single point = spot removal. |
| `source_offset` | `[dx,dy]` | Source = dest + offset for every stamp (normalized). |

### 5.11 `GeometryParams` (stage 9)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable. |
| `crop` | `CropRect` or null | null | Crop in normalized [0,1] coordinates of the rotated, perspective-corrected frame; null = full frame. `{ "x0", "y0", "x1", "y1" }` with 0 ≤ x0 < x1 ≤ 1, 0 ≤ y0 < y1 ≤ 1. |
| `rotate` | f32 | 0 | Degrees CCW, −45..=+45. |
| `perspective` | map | all 0 | `{ "vertical": f32, "horizontal": f32 }`, keystone amounts −100..=+100. |
| `flip_horizontal` | bool | `false` | Horizontal mirror. |

### 5.12 `FinishingParams` (stage 10)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `enabled` | bool | `true` | Master enable. |
| `vignette` | `VignetteParams` | see below | Post-crop vignette. |
| `grain` | `GrainParams` | see below | Procedural grain. |

`VignetteParams` — `amount` (f32 −100 darken..=+100 lighten, default 0 =
off), `midpoint` (f32 0..=100, default 50), `roundness` (f32 −100
rectangular..=+100 circular, default 0), `feather` (f32 0..=100, default
50).

`GrainParams` — `amount` (f32 0..=100, default 0 = off), `size` (f32
0..=100, default 25), `roughness` (f32 0..=100, default 50), `seed` (u64,
default 0). Grain is procedural and seeded, so identical sidecars render
identical grain.

### 5.13 `LiveIndex`

The directory view builds its entire index by scanning this block across
sidecars — nothing else (architecture.md §11).

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `rating` | u8 | 0 | Star rating, 0–5. |
| `flag` | text | `"None"` | `"None"`, `"Pick"`, or `"Reject"`. |
| `label` | String or null | null | Colour label name (e.g. `"Red"`). |
| `capture_time` | String or null | null | Cached capture time from raw metadata; RFC 3339 when known. |
| `thumbnail_hash` | bytes(32) or null | null | SHA-256 of the last-rendered thumbnail, as a 32-byte string (major type 2), for cache validation. |

### 5.14 `ExportRecipe`

A recipe records **every option that affects output bytes**, explicitly:
re-running a recipe against the same raw + edit + pipeline version
reproduces the exported file bit-identically.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `name` | String | `""` | User-visible recipe name. |
| `format` | `ExportFormat` | 16-bit deflate TIFF | Format + complete encoder options (below). |
| `color` | `ExportColor` | sRGB | Output colour options. |
| `hdr` | `HdrOptions` or null | null | null = SDR export. |
| `resize` | `ResizeSpec` or null | null | null = native (post-crop) resolution. |

`ExportFormat` variants (one-entry map):

- `{ "Tiff16": { "compression": TiffCompression } }` — 16-bit TIFF, the
  designated hand-off format. `TiffCompression`: `"None"`, `"Deflate"`
  (default), or `"Lzw"`.
- `{ "Png": { "bit_depth": u8 } }` — bits per sample, 8 or 16.
- `{ "Jpeg": { "quality": u8 } }` — baseline JPEG (always 8-bit),
  quality 1–100.
- `{ "JpegXl": { "distance": f32, "bit_depth": u8 } }` — Butteraugli
  distance (0.0 = mathematically lossless, 1.0 ≈ visually lossless,
  larger = smaller files); bits per sample 8 or 16.
- `{ "Avif": { "quality": u8, "bit_depth": u8 } }` — quality 1–100
  (100 = best); bits per sample 8, 10, or 12.

`ExportColor` — `{ "gamut": ExportGamut }`. `ExportGamut`: `"Srgb"`
(default), `"DisplayP3"`, `"AdobeRgb"`, `"Rec2020"`. These map 1:1 to
`focale_core::color::Gamut` at export time; the schema keeps its own enum
so the file format is decoupled from core internals.

`HdrOptions`:

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `transfer` | text | `"Pq"` | `"Pq"` (SMPTE ST 2084) or `"Hlg"` (ARIB STD-B67). |
| `peak_nits` | f32 | 1000 | Mastering peak luminance, cd/m². |
| `gain_map` | `GainMapOptions` or null | null | Gain-map request. **Seam only in v1**: an empty map `{}`; recipes may carry the block, but v1 execution rejects it (architecture §7). Fields arrive with the feature. |

`ResizeSpec` — `{ "long_edge": u32 }`: target length of the longer output
edge in pixels. Never upscales; values at or above the native long edge
leave the image unresized.

## 6. Writing safely

The reference implementation writes sidecars atomically: bytes go to a
temporary file in the same directory (`<name>.<pid>.tmp`), the file is
synced, and then renamed over the destination. Third-party writers should
do the same so a concurrently scanning reader never observes a truncated
document.

## 7. Conformance checklist for a third-party reader

1. Accept the tag-55799 envelope and its absence.
2. Accept any well-formed CBOR, not just CDE.
3. Ignore unknown map keys everywhere; default missing fields per §5.
4. Reject `schema_version` values greater than the newest implemented
   schema; read all older schemas forever.
5. Treat `pipeline_version` as opaque provenance: rendering an edit
   requires the matching pipeline algorithms.
6. Decode byte-string fields (`deflate_bitmap`, `thumbnail_hash`) as CBOR
   major type 2; also accept integer arrays only if you choose to be
   liberal — Focale never writes them.
7. Treat `focale_version` and `focale_platform` as opaque debug strings:
   never parse them for behaviour. When writing, stamp your own writer
   identification (or null) on every save.
