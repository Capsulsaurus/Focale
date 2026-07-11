# Focale

Mathematically-guided RAW image processor.

## Why This Exists

Professional raw developers (Lightroom, Capture One, DxO PhotoLab) produce deliverable
work — at the cost of subscriptions, sprawling panels, and workflows built for a slower
era. One-click tools trade away the decisions; Darktable is free but its depth is the
obstacle. Nothing serves the photographer with a trained eye who wants deliverable
results quickly, without babysitting fifty sliders per frame.

**[Name] is an AGPL raw developer built for speed with intent.** One correctness-ordered
pipeline, stripped of redundant controls: optics, then colour-managed development, then
finishing. Creative range lives in masks — geometric and AI-segmented, at parity with
the tools you're leaving — not in panel sprawl. Every edit is deterministic: the
pipeline is versioned, sidecars are canonical CBOR, and a file you develop today renders
bit-identically on any machine, offline, in ten years. Old sidecars never break. Ever.

**v1:** raw decode, optical corrections (embedded metadata first — we warn, never guess),
full tonal & colour toolkit, smart + standard masks, heal/dust retouch, batch apply
across frames, single-directory sessions (bring your own DAM), colour-managed preview
(sRGB / Display P3 / Adobe RGB aware), and export to TIFF, JXL, AVIF, PNG, JPEG with
full HDR where the format allows. **v2:** AI-suggested slider values, our own lens
measurement kit + open profile database, neural denoise/sharpen.

Success bar: 20 culled frames, finished in 30 minutes, indistinguishable from your
current editor's output.

## Guiding Principles

- Color representation: Mathematically model and expose images in physically faithful model.
- Offer tools to extend photos with creative freedom
- User-driven design: Everything you see feels like you can touch it. Responsive. Interactive. Intuitive.

## Implementation Guidelines

- Lean on open-source dependencies where possible and hand-roll for things missing. The upcoming `gamut` ecosystem should be able to handle majority of complexities in codebase.
- UI is designed to be consistent with common applications. No user guide should be necessary to explain any new feature.
- Assume hardware is capable and scale upwards.
- Use Rust where possible for its memory guarantees and modern toolchain. Drop to C/C++ for native APIs if strictly necessary. Compile with LLVM for all targets.
- Subsystems clearly define ownership of logic.
