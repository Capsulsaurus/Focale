# Application model

The normative UI/session contract: directory sessions, the editor, culling,
batch, and the status bar. Owning code: `focale-app` (`app`, `session`, `panels`,
`export_queue`, `jobs`). Governing invariants: `[HARD-FS]`, `[HARD-DET]`
([invariants](../invariants.md)).

## Session model (HARD)

Browsing is strictly one directory at a time. Opening a directory shows a
filmstrip of its raws (thumbnails, flags/ratings from sidecars). No recursion, no
collections, no database (`[HARD-FS]`). The directory view builds its entire
index by scanning sidecar live-index blocks ([sidecar](sidecar.md) §5.13) — file
names and directory shape carry no meaning.

## Editor

Single-image view with the fixed pipeline presented as ordered panels matching
the [pipeline](pipeline.md) stage order. No panel reordering.
**Redundant/duplicate controls are forbidden — one way to do each thing.**

## Culling & XMP interop

Culling metadata is rating (0–5), flag (pick/reject), and colour label, stored in
the `.fcl` live-index block ([sidecar](sidecar.md) §5.13). Keyboard culling
(`1–5`/`P`/`X`/`U`) is `v1 (shipped)`.

**The `.fcl` sidecar is the single source of truth for culling metadata —
permanently.** XMP interop is `v2 (committed)`
([scope](../scope.md#v2-committed)) and is strictly derived:

- **One-way mirror out:** on every culling change, Focale (re)writes a standard
  XMP sidecar (`<basename>.xmp`) next to the image: `xmp:Rating` always; reject
  flag as `xmp:Rating = -1` (the nearest thing to a cross-tool convention);
  colour label as English-canonical `xmp:Label` best-effort. The mirror is
  derived output — Focale never reads it back, so there are never two live
  sources of truth.
- **One-time import:** when opening a raw that has an XMP sidecar but **no**
  `.fcl` yet, Focale adopts rating/label (and `Rating = -1` as reject) into the
  new `.fcl` — culling done in other tools carries over exactly once, at
  adoption time.
- **Deliberately not more than this.** Load-bearing finding (verified 2026-07):
  cross-editor XMP culling interop is inconsistent, so restricting our sidecar
  to XMP's model would buy nothing — only `xmp:Rating` round-trips reliably;
  `xmp:Label` is free-text and localized (Capture One *writes* `xmp:Label` but
  *reads* the older `photoshop:Urgency`); Lightroom Classic wrote no pick/reject
  flags to XMP before 13.2 (2024) and its flag fields are not honored by
  Bridge or Capture One, which has no flag concept. Hence: featureful `.fcl` as
  SoT, simple standard mirror for everyone else.

## Batch (HARD)

(a) copy settings from one image and paste to a multi-selection; (b) multi-select
in the filmstrip while previewing one frame — edits save to every selected file's
sidecar. Plus a background export queue for multi-image export.

## Status bar (HARD)

Persistent, keyed fields including at minimum: active rendering gamut
([color](color.md)), pipeline version of the open file (with the older-version
warning and the explicit "upgrade to current pipeline" action — the only
operation that re-stamps a sidecar's version, [sidecar](sidecar.md) §3.3), image
colour info under cursor, zoom, and warnings (e.g. missing optics metadata,
[optics](optics.md)).

## AI-suggestion hook

v1 ships no suggestion model, but the UI and compute scheduler implement the
intended behavior as a stub ([preview](preview.md)).
