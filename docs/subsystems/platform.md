# Platform, stack & targets

Targets, GUI stack, performance targets, and application distribution. Owning
code: `focale-app` (GUI shell), CI workflows. Governing invariants:
`[HARD-RUST]`, `[HARD-LICENSE]`, `[HARD-LOCAL]` ([invariants](../invariants.md)).

## Targets (HARD)

macOS (Apple silicon) and Linux/Wayland, first-class. The stack must not preclude
Windows/X11, but no effort is spent on them; Windows promotion to first-class is
`demand-driven` ([scope](../scope.md#potential-extensions-given-demand)).
Current reality: Linux is exercised by CI on both architectures; macOS has no CI
and no platform-specific code yet — `v1 (gap, issue #10)`.

## GUI stack

`eframe` (currently 0.35 = `winit` 0.30 + `wgpu` 29 + `egui` 0.35). Rationale:
the image viewport must be a custom colour-managed render pass under our control
([color](color.md)), which rules out webview stacks; egui rides the same wgpu
surface for panels and keeps the whole app in Rust. UI chrome does not require
colour precision; the viewport shader does.

**Colour-management capability per target:** owned by the capability matrix in
[color](color.md#wide-gamut-display-capability-matrix) — summary: wide-gamut
display output is supportable on macOS (Metal) and Linux/**Wayland** (pending the
stack's move to wgpu ≥ 30); Linux/X11 is incapable, permanently.

## Performance target

Slider-to-screen update < 100 ms at fit-to-window zoom on a base Apple-silicon
Mac; full-resolution CPU export may be slower — correct beats fast on the export
path.

**Measured 2026-08-14 and not met**: a rich edit costs 234–480 ms in the CPU
pipeline alone, 2.4×–5.2× the budget, before any GPU work
([verification](../verification.md#preview-latency-measured) has the numbers,
the method, and the reproduction command). The target stands as a target; the
work to reach it is per-stage caching in the preview scheduler
([preview](preview.md)). Apple-silicon numbers wait on issue #10 — the figures
above are the Linux reference machine.

## Distribution & packaging

Per-target packaging decisions, recorded now so releases don't relitigate them.
Common constraints: AGPL-3.0 distribution duties (`[HARD-LICENSE]` — source offer
travels with every artifact), no store may inject networking/telemetry
(`[HARD-LOCAL]`), and the app must reach arbitrary user directories
(`[HARD-FS]`/session model — [app](app.md)).

- **Linux: Flatpak** (Flathub). Deciding factors: the only format that ships our
  exact runtime (no glibc/Mesa matrix), first-class Wayland, and Flathub is
  where Linux creative-app users already look. Sandbox note: raw directories are
  opened via the file-chooser **portal**, which grants real filesystem paths per
  session; persistent access to user-chosen photo roots uses
  `--filesystem` overrides the user controls. The portal flow must never be
  "solved" by a database of imported copies — that would violate `[HARD-FS]`.
  The one sanctioned download path (the model-fetch script,
  [rnd/ml-models.md](../rnd/ml-models.md)) runs on the host, outside the app, so
  the Flatpak manifest requests no network permission at all.

  **Verified 2026-07-20 — how `[HARD-LOCAL]` is expressed to Flathub.** There
  is no way to *declare* "no network": writing `--unshare=network` is a
  `flatpak-builder-lint` **error** (`finish-args-has-unshare-network`), because
  negated permissions are redundant against a sandbox that already denies by
  default. No-network is expressed by **omitting `--share=network` from
  `finish-args`** — which is what Focale does, and it is what users see on the
  listing. Flathub has **no written policy** on applications downloading assets
  post-install (the docs set was read in full; the absence is the finding), and
  runtime downloads are permitted in practice — but they require
  `--share=network`, which would publish a "Network access" permission that
  contradicts `[HARD-LOCAL]` regardless of what the code does. That is the
  reason the fetch path stays outside the app.

  **Preferred mechanism for weights under Flatpak: `extra-data`.** Flatpak
  itself fetches each artifact at *install* time from a URL pinned by `sha256`
  + `size`, before the app runs — so the app ships with no network permission
  and stays provably offline, while hash pinning is preserved exactly as
  [rnd/ml-models.md](../rnd/ml-models.md) requires. Flathub's repo-size ceiling
  is 12 GiB, far above our needs. Caveat to settle in the submission PR:
  `extra-data` is documented around *non-redistributable* and multi-GB sources,
  so for redistributable weights it is a size/UX choice rather than an
  obligation, and no doc addresses that case.

  Rejected: AppImage (no
  runtime isolation, glibc pinning pain, weak update story), Snap
  (Canonical-controlled store, poor fit for a non-Ubuntu audience), raw tarball
  (kept only as a CI artifact, unsupported).
- **macOS: DMG on GitHub Releases + Homebrew cask.** A cask (not a formula) is
  the correct Homebrew artifact for a GUI `.app`; it points at the same DMG.
  Served initially from a project-owned tap (`Capsulsaurus/homebrew-focale`).

  **Corrected 2026-07-20 — signing, not notability, is the binding
  constraint.** The notability thresholds are real (75 stars / 30 forks / 30
  watchers, or **225 stars / 90 forks / 90 watchers for a self-submission by
  the repo owner**, per `docs.brew.sh/Package-Acceptance-Policy`), but they
  are moot: Homebrew 5.0.0 (2025-11-12) deprecated unsigned casks and will
  **disable all `homebrew-cask` casks that fail Gatekeeper checks in
  September 2026**, reaffirmed in 6.0.0 (2026-06-11). `--no-quarantine` is
  deprecated, and Acceptable-Casks forbids requiring Gatekeeper to be
  bypassed. **An unsigned Focale cannot enter `homebrew-cask` at any star
  count.** Gatekeeper friction is also worse than recorded: control-click-to-open
  was removed in macOS 15 and not restored, so users must go to System
  Settings → Privacy & Security → Open Anyway, and macOS 26 often shows a
  misleading "app is damaged" dialog instead.

  This makes Developer ID signing + notarization (**$99/yr**) a decision the
  project must take before September 2026, not an optional polish item.
  Unsigned-viable alternatives if that fee is declined: **MacPorts** (no
  notability bar, source-built, signing irrelevant — the best fit for an AGPL
  Rust workspace), Nix/nix-darwin, a self-hosted tap (document the
  fully-qualified `brew install --cask Capsulsaurus/focale/focale` to sidestep
  Homebrew 6.0.0 Tap Trust), or a direct DMG with documented `xattr` removal.

  Rejected: Mac App Store — sandbox entitlements fight the
  arbitrary-directory session model, and store terms sit uneasily with AGPL.
- **Windows 10+ (when demand triggers it): MSI + winget.** MSI built with
  cargo-wix; a winget manifest in `microsoft/winget-pkgs` pointing at the
  GitHub-released MSI. Code-signing certificate needed to avoid SmartScreen
  friction. Rejected: MSIX/Microsoft Store (packaged-app sandbox + store terms,
  same concerns as macOS App Store).

All installers/artifacts are produced by CI from tagged releases
(release-plz-driven; see repo `README.md`), never hand-built.
