# Focale

Mathematically-guided RAW image processor.

## Why This Exists

Professional raw developers (Lightroom, Capture One, DxO PhotoLab) produce deliverable
work — at the cost of subscriptions, sprawling panels, and workflows built for a slower
era. One-click tools trade away the decisions; Darktable is free but its depth is the
obstacle. Nothing serves the photographer with a trained eye who wants deliverable
results quickly, without babysitting fifty sliders per frame.

**Focale is an AGPL raw developer built for speed with intent.** One correctness-ordered
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

## Support Matrix

- Platforms: macOS (Apple Silicon), Windows (x86\_64, arm\_64), Linux (Wayland) (x86\_64, arm\_64)*
- GPUs:
  * macOS: Apple Silicon (i.e. no external GPUs)
  * Windows: NVIDIA, AMD
  * Linux: VA-API (includes AMD and Intel via mesa drivers)
- Formats**: All common image formats and some RAW image formats are currently supported. We have explicit software support by specific devices. All support is done by an underlying open-source library called `rawshift`. The exact support is best referred to the comprehensive table: <https://github.com/justin13888/rawshift/tree/master/crates/rawshift-image#format-support>

> Don't see the format or camera that you need? Open a GitHub [issue directly in this repo (and not upstream)](https://github.com/Capsulsaurus/Focale/issues/new/choose)

*: Linux version strictly requires a reasonably recent version of Wayland and compositor with Wayland Color Management protocol

## Development

### Guiding Principles

- Color representation: Mathematically model and expose images in physically faithful model.
- Offer tools to extend photos with creative freedom
- User-driven design: Everything you see feels like you can touch it. Responsive. Interactive. Intuitive.

### Implementation Guidelines

- Lean on open-source dependencies where possible and hand-roll for things missing. The upcoming `gamut` ecosystem should be able to handle majority of complexities in codebase.
- UI is designed to be consistent with common applications. No user guide should be necessary to explain any new feature.
- Assume hardware is capable and scale upwards.
- Use Rust where possible for its memory guarantees and modern toolchain. Drop to C/C++ for native APIs if strictly necessary. Compile with LLVM for all targets.
- Subsystems clearly define ownership of logic.

### Getting Started

```bash
just run                     # launch the desktop app (Wayland/X11)
scripts/fetch-models.sh      # optional: download the AI segmentation models
cargo run -p focale-cli -- render photo.ARW --format tiff16   # headless export
```

Open a directory of raws (Sony lossless-compressed ARW or DNG in v1); cull with
`1–5`/`P`/`X`/`U` and the arrow keys; edit with the ordered stage panels; multi-select
in the filmstrip to broadcast edits or use *Copy settings → Paste to selection*;
export runs in a background queue (`focale-export/` beside your raws). Edits live in
`<file>.<ext>.fcl` sidecars — raws are never modified, and identical sidecars render
bit-identically on any machine, forever. See the docs index at `docs/README.md`
(subsystem specs, glossary) and the sidecar format in `docs/subsystems/sidecar.md`.

### Prerequisites

- [Rust (rustup)](https://rustup.rs) — toolchain (pinned via `rust-toolchain.toml`)
- [just](https://github.com/casey/just) — command runner
- [Lefthook](https://github.com/evilmartians/lefthook) — git hooks manager (`lefthook install` after cloning)
- [convco](https://github.com/convco/convco) — conventional-commit checker used by hooks
- cmake + a C++ toolchain — builds the vendored libjxl for JPEG XL export

## Commands

| Command      | Description                                  |
| ------------ | -------------------------------------------- |
| `just check` | Run everything CI runs (format, lint, tests) |
| `just test`  | Run the test suite                           |
| `just fmt`   | Format code                                  |
| `just lint`  | Clippy with warnings denied                  |
| `just run`   | Launch the desktop app                       |

## Git Hooks

This project uses Lefthook. Pre-commit auto-formats staged Rust files; commit-msg
validates the message is a conventional commit; pre-push runs the full CI check suite
(format, clippy, tests, commit-range check) so pushes never fail CI.

## CI/CD

GitHub Actions runs format checks, clippy, tests on pushes to `master` and pull
requests, plus conventional-commit validation on pull requests. A separate
Determinism workflow renders the committed (raw + sidecar) fixture on x86_64 and
aarch64 in every export format and fails if any byte differs (`docs/verification.md`).

## Releases & Changelog

Releases are automated via [release-plz](https://github.com/release-plz/release-plz):
a standing pull request tracks the next version bump; merging it tags the release and
updates `CHANGELOG.md` (generated from Conventional Commits). Commit messages must
follow [Conventional Commits](https://www.conventionalcommits.org/) — enforced by
`convco` on commit, pre-push, and in CI.

## License

AGPL-3.0 — see [LICENSE](LICENSE) for details. External contributions require a CLA
assigning rights to the project author.
