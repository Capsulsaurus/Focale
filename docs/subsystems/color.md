# Colour management

All colour science: primaries/transfer math, the colour-managed preview, the
HDR→SDR tone map, gamut mapping, and the single home of the per-platform
wide-gamut display capability matrix. Owning code: `focale-core/src/color`,
`focale-app/src/viewport` (display transform, WGSL mirror). Governing
invariants: `[HARD-DET]` ([invariants](../invariants.md)).

## Foundations

All primaries/transfer math is implemented in `focale-core::color` from the
published primaries (sRGB/Rec.709, Display P3, Adobe RGB, Rec.2020) with
Bradford chromatic adaptation; unit-tested against known reference values
([verification](../verification.md)). The working space itself (linear Rec.2020,
f32, unbounded) is defined with the pipeline ([pipeline](pipeline.md)).

## Colour-managed preview

- **Preview (HARD):** the app renders colour-managed; the image viewport is our
  own wgpu render pass whose fragment shader converts working-space linear
  Rec.2020 → the display space. Assume professional users on ~Display P3 D65
  hardware, but never hard-code the assumption. **Current reality**
  (`v1 (gap, issues #6/#10)`): v1 assumes an sRGB surface (the shader
  sRGB-encodes when the swapchain format is not already sRGB); wide-gamut
  surface output is unimplemented. The seam is the viewport uniform block, which
  already receives every colour matrix from `focale_core::color` constants.
- **HARD:** the GUI has a user-selectable **active rendering gamut** (sRGB,
  Display P3, Adobe RGB), always visible as a status-bar key ([app](app.md)).

## Wide-gamut display capability matrix

*(Single source of truth — [platform](platform.md) links here, never restates.)*

**Verified 2026-07, not yet wired:** wgpu 30.0.0 (2026-07-01) added
`SurfaceConfiguration::color_space` + `SurfaceCapabilities::format_capabilities`,
letting the swapchain be configured as e.g. `SurfaceColorSpace::DisplayP3`.
Per-platform reality:

- **macOS (Metal):** the broadest support in wgpu — `DisplayP3`,
  `ExtendedDisplayP3`, extended-linear (EDR), and BT.2100 PQ/HLG. This is the
  "tagged `CAMetalLayer`" requirement, satisfied through the same wgpu API.
- **Linux/Wayland:** the full chain ships today: Mesa ≥ 25.1 implements
  `VK_EXT_swapchain_colorspace` (+ `VK_EXT_hdr_metadata`) in its Wayland Vulkan
  WSI by speaking `wp_color_management_v1` (wayland-protocols ≥ 1.41 staging,
  Feb 2025) to the compositor — the protocol is handled by the driver on our
  `wl_surface`, so no winit support is required. Compositors shipping the
  protocol: GNOME 48+ (mutter), KDE Plasma 6.3+ (KWin), wlroots 0.19+ (Sway et
  al.), Hyprland. **Verified claim: wide-gamut (Display P3) preview output on
  Linux is supportable — on Wayland only.**
- **Linux/X11: incapable, permanently.** X.org has no colour-management protocol
  and Mesa's X11 WSI exposes only sRGB; the `x11` cargo feature exists solely so
  the binary can compile there (not targeted, not recommended, sRGB forever —
  [platform](platform.md)).
- **Blocker:** eframe/egui 0.35 (latest as of July 2026) pins wgpu 29, which
  predates the colour-space API; wiring this waits on egui's wgpu 30 upgrade
  (issues #6/#10). Integration note for then: one surface has one colour space,
  so when the surface is not sRGB, egui chrome (authored in sRGB) must be
  converted to the surface space in the render pass — chrome needs no colour
  *precision* ([platform](platform.md)), but it does need the correct primaries.

## HDR→SDR tone mapping *(pipeline-versioned)*

Extended Reinhard (white-point-preserving) on max-RGB in linear light. This
operator is a residual safety net that runs *after* the user has manually set
exposure and tone — the artistic decision is the user's; the operator only
disposes of remaining energy above 1.0 gracefully. For that role max-RGB extended
Reinhard is the right choice: one scale factor per pixel preserves channel ratios
(hue) exactly with zero trig; output is bounded in **every** channel, so the
gamut mapper only ever handles primaries mismatch, never tone overflow; it is a
few flops with no transcendentals, trivially deterministic, and exactly mirrored
in WGSL.

Trade-offs, stated honestly: the curve compresses everywhere, not just highlights
(mid-grey 0.18 → ≈0.154 at white=4, ≈14% darkening — visible in preview, so users
compensate with exposure, identically in preview and export), and saturated
colours render darker than a luminance-driven operator would.

Alternatives: BT.2446 Method C rated best in a 2025 subjective study (MDPI
Electronics 14(12):2428) — but for *unsupervised HDR-video→SDR broadcast
conversion of display-referred masters*, a different job; it is luminance-driven
(individual channels can exceed the mapped peak) and needs a crosstalk matrix +
Yxy round-trip. It remains the candidate if unsupervised batch conversion ever
becomes a priority (new pipeline version; `demand-driven`,
[scope](../scope.md#potential-extensions-given-demand)). ACES RRT, AgX, and Hable
were rejected for imposing a *look* (hue skews / filmic contrast) — wrong for a
neutral safety net.

## Gamut mapping *(pipeline-versioned)*

Hue-preserving chroma compression in Oklab — binary search on the (a,b) scale at
constant L, fixed 20 iterations, no trig on the mapping path. This is the same
geometry as the W3C-standardized CSS Color 4 §13 gamut-mapping algorithm (chroma
reduction in Oklch at constant lightness and hue); our fixed-iteration bisection
(chroma precision 2⁻²⁰) is better for determinism than CSS's ΔE-threshold
termination. Considered and rejected: CSS's MINDE step (accept a channel-clip
when ΔEok < 2) — trades back hue exactness and adds a discontinuity; CUSP
projection in JzAzBz/ICtCp — useful when tone and gamut are mapped jointly,
unnecessary here because the tone map has already bounded every channel, and both
spaces put a PQ nonlinearity (transcendental-heavy) on the mapping path.

## GPU mirror

The viewport WGSL mirrors the CPU operators (`map_to_gamut` and `tonemap` in
`shader.wgsl`), receiving every matrix from `focale_core::color` constants. The
contract is perceptual fidelity, not bit-identity (GPU filtering and precision
differ). No automated CPU-vs-WGSL parity test exists — accepted risk, reviewed on
any operator change.

## Deferred seam

Soft-proofing and print intent (`eventually`, [scope](../scope.md#eventually)):
the colour module is structured so a proofing transform can be inserted before
the display transform later; the feature is not built.
