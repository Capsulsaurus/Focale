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
- **Rec. 2020 is an export gamut but not a rendering gamut**, and the
  asymmetry is intentional. Export targets a *file*, whose consumers include
  HDR containers that are Rec. 2020-based by construction
  ([pipeline](pipeline.md)). Preview targets a *display*, and no consumer
  display renders Rec. 2020 — offering it would show the user a
  gamut-mapped-down image while labelling it Rec. 2020, which is precisely
  the lie colour management exists to prevent. Soft-proofing an export gamut
  is a different feature with a different UI, deferred below.
- **When the selected rendering gamut exceeds the surface**, the viewport
  renders to the widest space the surface actually supports and the status-bar
  key reports what is really being shown, not what was requested. The
  selection is a preference, never a claim; silently rendering P3 numbers into
  an sRGB surface would be the same lie in the other direction.

## Wide-gamut display capability matrix

*(Single source of truth — [platform](platform.md) links here, never restates.)*

**Verified 2026-07, not yet wired:** wgpu 30.0.0 (2026-07-01) added
`SurfaceConfiguration::color_space` + `SurfaceCapabilities::format_capabilities`,
letting the swapchain be configured as e.g. `SurfaceColorSpace::DisplayP3`.
Per-platform reality:

- **macOS (Metal):** the broadest support in wgpu — `DisplayP3`,
  `ExtendedDisplayP3`, extended-linear (EDR), and BT.2100 PQ/HLG. This is the
  "tagged `CAMetalLayer`" requirement, satisfied through the same wgpu API.
- **Linux/Wayland:** the full chain ships today. Mesa ≥ 25.1 (2025-05-07)
  implements the Wayland colour-management protocol in its Vulkan WSI —
  `wsi_common_wayland.c` binds `wp_color_manager_v1` against the swapchain's
  own `wl_surface`, maps `VkColorSpaceKHR` values onto protocol primaries and
  transfer functions, and filters the advertised list against what the
  compositor supports. **The driver handles the protocol on our surface, so no
  winit support is required** — and winit has none: issue
  [#4131](https://github.com/rust-windowing/winit/issues/4131) has been open
  and untouched since Feb 2025. Colour space is selected through the Vulkan
  surface, not the windowing library.
  - Protocol: `wp-color-management-v1` landed in **wayland-protocols 1.41**
    (tagged 2025-02-17) and **is still `staging/` as of 1.49** (2026-06-07) —
    ~16 months without promotion to `stable/`. Do not write specs that assume
    near-term stabilisation.
  - Compositors, by first shipping release (verified by protocol symbols at
    release tags, 2026-07-20): **GNOME/Mutter 48.0**, **KDE Plasma 6.4.0**
    (2025-06-17 — *not* 6.3), **wlroots 0.19.x** / **Sway 1.12**,
    **Hyprland 0.48.0**. The older `xx-color-management-v4` and
    `frog-color-management-v1` are dead: Hyprland dropped both in 0.53, and
    KWin and Mutter carry neither today. `wp-color-management-v1` is the
    single universal target.
  - **Verified claim: wide-gamut (Display P3) preview output on Linux is
    supportable — on Wayland only.**
  - *Unverified:* whether Mesa advertises `VK_EXT_swapchain_colorspace` /
    `VK_EXT_hdr_metadata` on a Wayland surface, and from which version per
    driver (RADV/ANV/NVK). The protocol plumbing is confirmed present from
    25.1; the extension-advertisement claim is not, and was previously
    asserted here without evidence.
- **Linux/X11: incapable, permanently.** X.org has no colour-management protocol
  and Mesa's X11 WSI exposes only sRGB; the `x11` cargo feature exists solely so
  the binary can compile there (not targeted, not recommended, sRGB forever —
  [platform](platform.md)).
- **Blocker (status verified 2026-07-20, changing):** released `egui-wgpu`
  0.35.0 (2026-06-25) depends on `wgpu ^29`, which predates the colour-space
  API — so the *released* stack cannot reach it. But egui PR
  [#8289](https://github.com/emilk/egui/pull/8289) ("Upgrade wgpu to v30")
  **merged to `main` on 2026-07-20**, so the fix exists upstream and is
  awaiting a release. Tracking issue
  [#8312](https://github.com/emilk/egui/issues/8312) remains open. Past
  release cadence (0.33.0 2025-10-09, 0.34.0 2026-03-26, 0.35.0 2026-06-25)
  is roughly quarterly, which would put 0.36 around Sept–Oct 2026 — an
  **extrapolation, not an announced date**; no maintainer has stated one.
  - **The pin cannot be routed around by dropping eframe.**
    `egui_wgpu::Renderer::new` takes `wgpu` types by reference, so the caller's
    `wgpu` must be the same semver-major as `egui-wgpu`'s. The pin lives in
    `egui-wgpu`, not `eframe`. The only way to have this before 0.36 is a git
    dependency on egui `main`.
  - Neither issue #6 nor #10 tracks the egui upgrade itself — they track the
    two platform halves that consume it ([scope](../scope.md)). The upstream
    bump has no Focale issue because it is not Focale work.
- Integration note for when it lands: one surface has one colour space, so
  when the surface is not sRGB, egui chrome (authored in sRGB) must be
  converted to the surface space in the render pass — chrome needs no colour
  *precision* ([platform](platform.md)), but it does need the correct primaries.

## HDR→SDR tone mapping *(pipeline-versioned)*

Extended Reinhard (white-point-preserving) on max-RGB in linear light, with
the white point — the linear input mapping to exactly 1.0 — pinned per
pipeline version at **4.0** (`focale_core::color::REINHARD_WHITE_DEFAULT`,
two stops above diffuse white). Export and the preview viewport read the same
constant, which is what makes them agree; changing it changes output bytes
and so requires a new pipeline version (`[HARD-VER]`). Exposing it as a
user-facing HDR-headroom control is specified as an additive recipe field in
[sidecar](sidecar.md) §5.14, deferred until a UI needs it. This
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

Alternatives, re-verified 2026-07-20:

**BT.2446 Method C** — correctly cited as **Report ITU-R BT.2446-1 (03/2021)
§6**. It is a *Report*, not a Recommendation, and -1 is the current revision
(there is no -2). It is display-referred by its own statement, luminance-driven
(tone mapping applies to Y only in Yxy, so individual channels can exceed the
mapped peak once x,y are reapplied), and needs a crosstalk matrix + Yxy
round-trip — a different job from ours: *unsupervised HDR-video→SDR broadcast
conversion of display-referred masters*.

A second, independent reason it cannot be adopted as-is, which is the stronger
one under `[HARD-DET]`/`[HARD-VER]`: **Method C is not a fully-pinned
algorithm.** Its published `k1 = 0.83802, k2 = 15.09968, k3 = 0.74204,
k4 = 78.99439` are explicitly an *example* set — the Report states that "a
different set of parameter values for k1 to k4 can be derived" per production
intent — and it leaves the crosstalk parameter **α** as a range (0 ≤ α ≤ 0.33)
with no default and the chroma-correction **σ** as a user parameter, with the
whole §6.1.8 correction optional. The real-world implementations diverge
accordingly: of the two inspected, one defaults α to 0 (no desaturation) and
neither implements σ. Adopting it would mean making those pinning decisions
ourselves. It remains the candidate if unsupervised batch conversion ever
becomes a priority (new pipeline version; `demand-driven`,
[scope](../scope.md#potential-extensions-given-demand)).

*Unverified claim, deliberately not asserted:* an earlier draft of this doc
said Method C "rated best in a 2025 subjective study (MDPI Electronics
14(12):2428)". That paper could not be retrieved (mdpi.com returns HTTP 403 to
automated fetches), so the claim is **unread and must not be relied on**. Two
things need checking before it is restored: what Method C was compared
*against* (if only against BT.2446 Methods A/B, "rated best" says nothing about
Reinhard), and which revision of BT.2446 it evaluated.

**ACES RRT, AgX, and Hable** were rejected for imposing a *look* (hue skews /
filmic contrast) — wrong for a neutral safety net. Two harder,
evidence-backed reasons now supersede the aesthetic argument:

- **AgX has no canonical definition to pin.** There is no specification, no
  reference constant table, and no conformance test — the origin repository is
  an "experimental configuration" with "aesthetic tunings". Blender does not
  ship the original author's AgX but a community fork, and delivers the
  picture-forming step as an opaque **3D LUT** (per-display `.cube` files),
  so reproducing it bit-exactly would mean shipping their LUT *and*
  reimplementing OCIO's tetrahedral interpolation. Four surveyed
  implementations (original, Blender fork, three.js 6th-order polynomial fit,
  darktable) are mutually incompatible. A name that does not designate a
  specific algorithm cannot satisfy `[HARD-VER]`.
- **ACES 2.0 changed pixel output without a version bump** — the 2025 release
  notes state the refactor "changes pixel output slightly" relative to the
  prior developer release, so "ACES 2.0" names several numerically distinct
  renderings and one would have to pin a commit, not a version. It also builds
  lookup tables at runtime and uses binary searches, and was measured 3–8×
  slower than ACES 1.

## Gamut mapping *(pipeline-versioned)*

Hue-preserving chroma compression in Oklab — binary search on the (a,b) scale at
constant L, fixed 20 iterations, no trig on the mapping path. This is the same
geometry as the CSS Color 4 gamut-mapping algorithm (chroma reduction in Oklch
at constant lightness and hue); our fixed-iteration bisection (chroma precision
2⁻²⁰) is better for determinism than CSS's ΔE-threshold termination.

**Citation corrected 2026-07-20** (verified against the CSSWG Bikeshed source,
`w3c/csswg-drafts` `css-color-4/Overview.bs`): the section is **§14**, not §13;
CSS Color 4 is a **Candidate Recommendation Draft (17 July 2026)**, not a W3C
Recommendation, so "W3C-standardized" overstated it; and §14.2 now offers
**three** algorithms (Binary Search with Local MINDE, EdgeSeeker, Ray Trace)
rather than one. That restructuring happened within days of this writing, which
*strengthens* the argument for not tracking it: our fixed-iteration bisection is
pinned where CSS's is actively moving. Treat the citation as "same geometry as"
a dated snapshot, never as an appeal to a stable standard.

Considered and rejected: CSS's MINDE step (accept a channel-clip when
**ΔEok < 0.02**) — trades back hue exactness and adds a discontinuity. *(The
threshold was previously recorded here as `< 2`, which was wrong by a factor of
100: 2 is the **CIE Lab ΔE2000** JND, and the spec notes that because Oklab
lightness ranges 0–1 rather than 0–100, "using deltaEOK, one JND is 100 times
smaller." The spec's algorithm pins `JND = 0.02` and `epsilon = 0.0001`. At
`< 2` essentially every clip would be accepted, so the rejection rationale
would not have held as written.)* Also rejected: CUSP
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
