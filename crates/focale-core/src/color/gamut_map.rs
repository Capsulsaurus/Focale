//! Gamut mapping: pipeline v1 operator (docs/subsystems/color.md).
//!
//! # Operator definition (v1 — frozen)
//!
//! Hue-preserving chroma compression in Oklab:
//!
//! 1. Convert linear Rec.2020 to the target gamut. If every component is
//!    within `[−ε, 1 + ε]` the colour is in gamut: return it clamped to
//!    `[0, 1]`.
//! 2. Otherwise convert to Oklab, hold lightness `L` and hue (the direction
//!    of the `(a, b)` vector) fixed, and binary-search the largest chroma
//!    scale `s ∈ [0, 1]` whose target-space RGB is in gamut. Exactly
//!    [`CHROMA_SEARCH_ITERATIONS`] bisection steps, keeping the lower
//!    (in-gamut) bound — fixed iteration count, deterministic.
//! 3. Clamp the result to `[0, 1]`.
//!
//! Scaling `(a, b)` directly keeps the hue constant by construction (no
//! trigonometry on the mapping path). Neutral colours have chroma ≈ 0 and
//! take the step-1 fast path, so blacks and whites pass through unchanged.
//! Changing any of this behaviour requires a new pipeline version
//! ([`crate::PIPELINE_VERSION`]).

use super::oklab::{oklab_to_xyz, xyz_to_oklab};
use super::primaries::{Gamut, REC2020_TO_XYZ, rec2020_to_gamut};

/// Fixed bisection count of the chroma search (v1 — frozen).
pub const CHROMA_SEARCH_ITERATIONS: u32 = 20;

/// In-gamut slack in linear RGB; components within this of [0, 1] count as
/// in gamut and are clamped (v1 — frozen).
const EPS: f32 = 1e-4;

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|c| (-EPS..=1.0 + EPS).contains(c))
}

fn clamp01(rgb: [f32; 3]) -> [f32; 3] {
    [
        rgb[0].clamp(0.0, 1.0),
        rgb[1].clamp(0.0, 1.0),
        rgb[2].clamp(0.0, 1.0),
    ]
}

/// Maps a linear Rec.2020 colour into `target`, returning linear
/// target-space RGB in `[0, 1]`.
///
/// Pipeline v1 gamut-mapping operator; see the module documentation for the
/// frozen definition.
pub fn map_to_gamut(rgb_rec2020_linear: [f32; 3], target: Gamut) -> [f32; 3] {
    let direct = rec2020_to_gamut(target).mul_vec(rgb_rec2020_linear);
    if in_gamut(direct) {
        return clamp01(direct);
    }

    let lab = xyz_to_oklab(REC2020_TO_XYZ.mul_vec(rgb_rec2020_linear));
    let xyz_to_rgb = target.xyz_to_rgb();
    let candidate = |s: f32| xyz_to_rgb.mul_vec(oklab_to_xyz([lab[0], lab[1] * s, lab[2] * s]));

    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    for _ in 0..CHROMA_SEARCH_ITERATIONS {
        let mid = 0.5 * (lo + hi);
        if in_gamut(candidate(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    clamp01(candidate(lo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::oklab_to_oklch;
    use crate::color::primaries::{SRGB_TO_REC2020, SRGB_TO_XYZ};
    use crate::color::testutil::assert_vec_close;

    fn oklch_of(rgb: [f32; 3], gamut: Gamut) -> [f32; 3] {
        oklab_to_oklch(xyz_to_oklab(gamut.rgb_to_xyz().mul_vec(rgb)))
    }

    fn hue_distance(a: f32, b: f32) -> f32 {
        use std::f32::consts::PI;
        let d = (a - b + PI).rem_euclid(2.0 * PI) - PI;
        d.abs()
    }

    #[test]
    fn in_gamut_colours_unchanged() {
        let srgb = [0.2_f32, 0.5, 0.8];
        let rec2020 = SRGB_TO_REC2020.mul_vec(srgb);
        assert_vec_close(map_to_gamut(rec2020, Gamut::Srgb), srgb, 1e-4);
    }

    #[test]
    fn black_and_white_preserved() {
        assert_eq!(map_to_gamut([0.0, 0.0, 0.0], Gamut::Srgb), [0.0, 0.0, 0.0]);
        let white = map_to_gamut([1.0, 1.0, 1.0], Gamut::Srgb);
        assert_vec_close(white, [1.0, 1.0, 1.0], 1e-4);
        assert!(white.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn identity_for_rec2020_target() {
        let rgb = [0.9, 0.1, 0.4];
        assert_vec_close(map_to_gamut(rgb, Gamut::Rec2020), rgb, 0.0);
    }

    #[test]
    fn rec2020_red_maps_into_srgb_preserving_hue() {
        let red = [1.0_f32, 0.0, 0.0];
        let mapped = map_to_gamut(red, Gamut::Srgb);
        assert!(
            mapped.iter().all(|c| (0.0..=1.0).contains(c)),
            "out of range: {mapped:?}"
        );
        assert!(
            mapped[0] > mapped[1] && mapped[0] > mapped[2],
            "not red: {mapped:?}"
        );

        let original = oklch_of(red, Gamut::Rec2020);
        let result = oklab_to_oklch(xyz_to_oklab(SRGB_TO_XYZ.mul_vec(mapped)));
        // Hue preserved within 1°.
        assert!(
            hue_distance(original[2], result[2]) < 1.0_f32.to_radians(),
            "hue moved: {} -> {}",
            original[2],
            result[2]
        );
        // Lightness preserved, chroma reduced but not destroyed.
        assert!((original[0] - result[0]).abs() < 1e-3);
        assert!(result[1] <= original[1] + 1e-4);
        assert!(result[1] > 0.05);
    }

    #[test]
    fn rec2020_green_maps_into_srgb_preserving_hue() {
        let green = [0.0_f32, 1.0, 0.0];
        let mapped = map_to_gamut(green, Gamut::Srgb);
        assert!(mapped.iter().all(|c| (0.0..=1.0).contains(c)));
        assert!(mapped[1] > mapped[0] && mapped[1] > mapped[2]);

        let original = oklch_of(green, Gamut::Rec2020);
        let result = oklab_to_oklch(xyz_to_oklab(SRGB_TO_XYZ.mul_vec(mapped)));
        assert!(hue_distance(original[2], result[2]) < 1.0_f32.to_radians());
        assert!(result[1] <= original[1] + 1e-4);
    }

    #[test]
    fn mapping_is_idempotent() {
        let red = [1.0_f32, 0.0, 0.0];
        let mapped = map_to_gamut(red, Gamut::Srgb);
        let again = map_to_gamut(SRGB_TO_REC2020.mul_vec(mapped), Gamut::Srgb);
        assert_vec_close(again, mapped, 1e-3);
    }

    #[test]
    fn maps_into_every_target() {
        let extreme = [1.2_f32, -0.1, 0.05];
        for gamut in Gamut::ALL {
            let mapped = map_to_gamut(extreme, gamut);
            assert!(
                mapped.iter().all(|c| (0.0..=1.0).contains(c)),
                "{gamut:?} produced {mapped:?}"
            );
        }
    }
}
