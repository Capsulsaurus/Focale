# Focale

> Status (July 2026): Alpha and actively developed. Star here on GitHub and stay tuned!

Mathematically-guided RAW image processor and editor.

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

## FAQ

**Q: Focale's workflow and capabilities do not cover what I need.**
A: If you have specific critique or suggestions, please feel free to submit issue for specific feature requests or open a conversation to discuss potential resolutions. We aim to rapidly address community needs although we may push back if there are technical reasons.

**Q: Why should I use Focale over <insert your existing editor here>**
A (from creator): There can be many reasons but my biggest reason is that none of the proprietary software I have ever creates edits remains replicable in 10+ years and across all devices. We are aiming to be the best open RAW image processor/editor suitable for (most) professional use.

**Q: If it were so capable, why is it free?**
A: It was created out of need and not as some company's core life blood. The power in a better product is that it is being used and adopted as a community.

**Q: The decoded images look different from what I expected.**
A: Assuming you have verified with another software, please file a bug report. It could be a device we did not test for.

**Q: I want to use another tool in conjunction.**
A: If you have another tool you need to further process your images, your best bet like other raw processors is to *export as TIFF* and continue. We do not plan to have a button along the lines "Open in <photo editor of choice>" as we do not endorse any company's paid products, each with quite different cataloguing ideologies.

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
mise run run                 # launch the desktop app (Wayland/X11)
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
- [mise](https://mise.jdx.dev) — task runner; it also pins and installs the tools below
- cmake + a C++ toolchain — builds the vendored libjxl for JPEG XL export (install separately)

After cloning:

```bash
mise install      # fetch the pinned tools (hk, convco)
mise run hooks    # install the git hooks
```

`mise.toml` pins [hk](https://hk.jdx.dev) (git hooks) and
[convco](https://github.com/convco/convco) (conventional-commit checker), so neither
needs installing by hand. `mise run hooks` installs the hooks into this clone only;
it runs them through `mise x`, so git must be able to find `mise` on its `PATH` —
if you commit from a GUI client with a trimmed environment, use
`hk install --global --mise` instead.

## Commands

`mise.toml` is the single source of truth for every command: the git hooks and
every CI job invoke these same tasks. `mise tasks ls` lists them all.

| Command               | Description                                  |
| --------------------- | -------------------------------------------- |
| `mise run check`      | Run everything CI runs (format, lint, tests) |
| `mise run test`       | Run the test suite                           |
| `mise run fmt`        | Format code                                  |
| `mise run lint`       | Clippy with warnings denied                  |
| `mise run run`        | Launch the desktop app                       |
| `mise run commits`    | Validate conventional commits in a range     |
| `mise run determinism`| Render the determinism fixture and hash it   |

## Git Hooks

This project uses [hk](https://hk.jdx.dev); `mise run hooks` installs them.
`hk.pkl` decides only *when* each task runs — the commands themselves come from
`mise.toml`.

- **pre-commit** formats staged Rust files. hk stashes unstaged work first, so the
  hook sees the staged content and only the files you staged get formatted and
  re-staged — work you deliberately left out of the commit is preserved untouched.
- **commit-msg** validates the message is a conventional commit; merge and rebase
  commits are exempt.
- **pre-push** runs the full CI check suite (format, clippy, tests, commit-range
  check) on every push, so pushes should not fail CI. The Determinism workflow is
  the exception — it is a two-architecture comparison CI alone can make.

## CI/CD

GitHub Actions runs format checks, clippy, tests and an `hk.pkl` validation on pushes
to `master` and pull requests, plus conventional-commit validation on pull requests.
Every job invokes the same `mise run <task>` a developer runs locally, so CI and the
git hooks cannot drift apart. A separate Determinism workflow renders the committed
(raw + sidecar) fixture on x86_64 and aarch64 in every export format and fails if any
byte differs (`docs/verification.md`).

## Releases & Changelog

Releases are automated via [release-plz](https://github.com/release-plz/release-plz):
a standing pull request tracks the next version bump; merging it tags the release and
updates `CHANGELOG.md` (generated from Conventional Commits). Commit messages must
follow [Conventional Commits](https://www.conventionalcommits.org/) — enforced by
`convco` on commit, pre-push, and in CI.

## License

AGPL-3.0 — see [LICENSE](LICENSE) for details. External contributions require a CLA
assigning rights to the project author.
