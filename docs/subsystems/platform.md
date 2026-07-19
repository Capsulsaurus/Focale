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
path. (Unmeasured; `v1 (gap, issue #11)` — [preview](preview.md).)

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
  the Flatpak manifest requests no network permission at all. Rejected: AppImage (no
  runtime isolation, glibc pinning pain, weak update story), Snap
  (Canonical-controlled store, poor fit for a non-Ubuntu audience), raw tarball
  (kept only as a CI artifact, unsupported).
- **macOS: DMG on GitHub Releases + Homebrew cask.** A cask (not a formula) is
  the correct Homebrew artifact for a GUI `.app`; it points at the same DMG.
  Served initially from a project-owned tap (`Capsulsaurus/homebrew-focale`) —
  official `homebrew-cask` inclusion has notability thresholds, revisit once
  met. Constraint recorded: distribution outside the App Store still requires
  Developer ID signing + notarization (annual Apple fee) or users face
  Gatekeeper friction. Rejected: Mac App Store — sandbox entitlements fight the
  arbitrary-directory session model, and store terms sit uneasily with AGPL.
- **Windows 10+ (when demand triggers it): MSI + winget.** MSI built with
  cargo-wix; a winget manifest in `microsoft/winget-pkgs` pointing at the
  GitHub-released MSI. Code-signing certificate needed to avoid SmartScreen
  friction. Rejected: MSIX/Microsoft Store (packaged-app sandbox + store terms,
  same concerns as macOS App Store).

All installers/artifacts are produced by CI from tagged releases
(release-plz-driven; see repo `README.md`), never hand-built.
