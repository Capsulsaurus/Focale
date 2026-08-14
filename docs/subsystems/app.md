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
- **The pick flag is deliberately not mirrored.** *(Decision unchanged;
  reasoning re-justified 2026-07-20, because the previous justification —
  that no pick field exists in XMP — is no longer true.)*

  A pick field now does exist: **Lightroom Classic 13.2 (Feb 2024) began
  writing flags to XMP as `xmpDM:pick`** (an integer; flagged = 1, unflagged
  = 0), confirmed in ExifTool's xmpDM table. What has *not* appeared is a
  portable convention. Four tools use four mutually-incompatible encodings,
  three of them in vendor-private namespaces:

  | Tool | Pick | Reject |
  | --- | --- | --- |
  | Lightroom Classic ≥ 13.2 | `xmpDM:pick = 1` | value unverified |
  | digiKam | `digiKam:PickLabel = 3` | `= 1` (plus `2` = pending) |
  | Photo Mechanic | `photomechanic:Tagged = True` | — (Color Class 8 by convention) |
  | Bridge / darktable | — | `xmp:Rating = -1` |
  | Capture One / RawTherapee | no flag concept | — (RT clamps −1 to 0) |

  Adobe's own release note is the decisive evidence: the feature is "primarily
  for compatibility with Lightroom Desktop, and **is not supported by other
  apps like Bridge**" — i.e. not portable even within Adobe's own product
  line. Writing `xmpDM:pick` would buy interop with exactly one application,
  in a namespace nominally for Dynamic Media, with an unverified reject value
  and a known Adobe bug (the flag is not written when `XMP:Pick` is absent
  from the file and nothing else changed). So pick stays `.fcl`-only; a pick
  survives a round-trip through another editor only as whatever rating the
  user also applied.

  Reject *is* mirrored (as `xmp:Rating = -1`) on stronger footing than
  "nearest thing to a convention": **−1 is normative**, defined in Adobe's
  XMP Basic namespace as part of `xmp:Rating` itself — "The value shall be −1
  or in the range [0..5], where −1 indicates 'rejected'" — and it is honoured
  by Bridge and darktable. Losing a reject silently is destructive in a way
  losing a pick is not.
- **Import is deliberately more liberal than export.** A one-time adoption has
  no round-trip obligation, so the importer reads every encoding it can
  recognise: `xmpDM:pick`, `digiKam:PickLabel`, `photomechanic:Tagged`, and
  `xmp:Rating ∈ {−1, 6}` as reject. The `6` is not a typo — darktable
  historically wrote `6` rather than `−1` for rejected, so old darktable
  sidecars in the wild encode reject that way. Liberal read, conservative
  write.
- **Colour label vocabulary.** The mirror writes exactly `Red`, `Yellow`,
  `Green`, `Blue`, `Purple` — English, capitalized, ASCII — because
  `xmp:Label` is free text matched by string, and localized or custom names
  are what break interop in practice. A label outside that set is written
  verbatim (it is the user's data), with the understanding that other tools
  will show it as an unrecognized label rather than a colour.

  *Evidence (2026-07-20):* this is the only five-value set appearing verbatim
  in three independent open-source implementations — RawTherapee's
  bidirectional map, darktable's reader, and digiKam's writer table — and it
  matches Lightroom's English default. Localized Lightroom installs
  demonstrably write localized strings ("Grün", "Rood"), which is exactly the
  interop breakage we avoid by always writing English regardless of UI
  language. digiKam additionally emits `Orange`/`Gray`/`Black`/`White`; those
  are safe to *read* but must not be written, since no other tool recognises
  them.

  **`photoshop:Urgency` is deliberately not written.** Capture One
  historically read it in preference to `xmp:Label`, but it is a priority
  scale (1–8, "should be in the range 1-8 to conform with the XMP spec")
  being abused as a colour enum; the circulated colour mapping is unverified
  and internally inconsistent (Pink and Purple both map to 5); and modern
  Capture One reportedly prefers `xmp:Label` anyway. Writing it would place a
  second encoding of the same fact in the file — the redundancy the editor
  rules out. If C1-legacy support is ever wanted it belongs behind an explicit
  opt-in, as FastRawViewer and Photo Mechanic both do, never as a default.

  **Asymmetry worth knowing for the importer:** darktable *reads*
  `xmp:Label` but **never writes it** — it writes `Xmp.darktable.colorlabels`
  (an integer sequence) instead. So nothing read back from a darktable-written
  sidecar will carry `xmp:Label`, and importing labels from darktable requires
  falling back to that integer field (and to `digiKam:ColorLabel` for digiKam).
- **Sidecar naming: `<basename>.xmp` (replace extension), not
  `<basename>.<ext>.xmp`.** Verified rationale: Lightroom, Bridge, and
  Capture One read *only* the replace-extension form, while darktable and
  digiKam read **both**. Replace-extension therefore strictly dominates — it
  is read by every tool surveyed. Two consequences to design around: (a)
  `IMG_1234.ARW` and `IMG_1234.JPG` in one directory collide on a single
  `IMG_1234.xmp`, which is a real ambiguity in the Adobe scheme and the reason
  darktable chose the other one — Focale must decide explicitly what it does
  there; (b) darktable will read our `.xmp` and then write its own
  `.ARW.xmp` alongside it, so a user running both tools ends up with two
  sidecars and ours goes stale. Acceptable for a one-way derived mirror, but
  it should be documented for users rather than discovered.
- **If the XMP write fails** (read-only directory, permissions, full disk),
  the culling change still commits to the `.fcl` and the failure surfaces as
  a non-blocking status-bar warning. The mirror is derived data; a
  filesystem that cannot take it must never cost the user the edit itself.
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

**What "settings" means:** the complete `EditState` ([sidecar](sidecar.md)
§5.2) — every stage's parameters, including masks and retouch strokes.
Deliberately all-or-nothing: a per-stage paste picker is the kind of
redundant control the editor rules out above, and mask/retouch coordinates
are normalized to the pre-geometry frame ([sidecar](sidecar.md) §4), so they
transfer meaningfully between frames of the same shoot. Culling metadata
(`LiveIndex`, §5.13) is *not* settings and is never copied — pasting someone
else's rating over a selection would be destructive. Paste never changes a
target's `pipeline_version`; §3.3's upgrade action remains the only thing
that does.

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
