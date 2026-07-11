//! HDR→SDR tone mapping: pipeline v1 operator (docs/architecture.md §7).
//!
//! # Operator definition (v1 — frozen)
//!
//! Extended Reinhard (Reinhard, Stark, Shirley, Ferwerda, "Photographic
//! Tone Reproduction for Digital Images", SIGGRAPH 2002, eq. 4) applied to
//! the maximum RGB component in linear light:
//!
//! ```text
//! m  = max(r, g, b)
//! m' = m · (1 + m / white²) / (1 + m)
//! out = rgb · (m' / m)
//! ```
//!
//! Driving the curve with max-RGB instead of luminance scales all three
//! channels by one factor, which preserves channel ratios (hue) and cannot
//! push any channel above the mapped peak. Inputs at `white` map to exactly
//! 1.0; inputs above `white` exceed 1.0 and are expected to be clamped or
//! gamut-mapped downstream. Changing any of this behaviour requires a new
//! pipeline version ([`crate::PIPELINE_VERSION`]).

/// Default white point for [`tonemap_reinhard_extended`]: linear 4.0
/// (two stops above diffuse white) maps to 1.0.
pub const REINHARD_WHITE_DEFAULT: f32 = 4.0;

/// Extended Reinhard tone map on max-RGB in linear light (v1 — frozen).
///
/// `white` is the linear input that maps to exactly 1.0 and must be finite
/// and > 0 (see [`REINHARD_WHITE_DEFAULT`]). Pixels whose maximum component
/// is ≤ 0 are returned unchanged.
pub fn tonemap_reinhard_extended(rgb: [f32; 3], white: f32) -> [f32; 3] {
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m <= 0.0 {
        return rgb;
    }
    let mapped = m * (1.0 + m / (white * white)) / (1.0 + m);
    let scale = mapped / m;
    [rgb[0] * scale, rgb[1] * scale, rgb[2] * scale]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::testutil::{assert_close, assert_vec_close};

    #[test]
    fn white_point_maps_to_one() {
        let w = REINHARD_WHITE_DEFAULT;
        assert_vec_close(
            tonemap_reinhard_extended([w, w, w], w),
            [1.0, 1.0, 1.0],
            1e-6,
        );
        // Only the max channel hits 1.0 for non-neutral input.
        let out = tonemap_reinhard_extended([w, 1.0, 0.5], w);
        assert_close(out[0], 1.0, 1e-6);
        assert!(out[1] < 1.0 && out[2] < 1.0);
    }

    #[test]
    fn small_values_stay_nearly_linear() {
        let out = tonemap_reinhard_extended([0.01, 0.005, 0.002], REINHARD_WHITE_DEFAULT);
        assert_vec_close(out, [0.01, 0.005, 0.002], 1e-3);
    }

    #[test]
    fn black_and_non_positive_unchanged() {
        assert_eq!(
            tonemap_reinhard_extended([0.0, 0.0, 0.0], REINHARD_WHITE_DEFAULT),
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            tonemap_reinhard_extended([-0.1, -0.2, 0.0], REINHARD_WHITE_DEFAULT),
            [-0.1, -0.2, 0.0]
        );
    }

    #[test]
    fn channel_ratios_preserved() {
        let input = [2.0_f32, 1.0, 0.25];
        let out = tonemap_reinhard_extended(input, REINHARD_WHITE_DEFAULT);
        assert_close(out[1] / out[0], input[1] / input[0], 1e-6);
        assert_close(out[2] / out[0], input[2] / input[0], 1e-6);
    }

    #[test]
    fn monotonic_in_peak() {
        let mut prev = -1.0_f32;
        for i in 1..=200 {
            let m = i as f32 * 0.05;
            let out = tonemap_reinhard_extended([m, m, m], REINHARD_WHITE_DEFAULT);
            assert!(out[0] > prev, "not monotonic at {m}");
            prev = out[0];
        }
    }
}
