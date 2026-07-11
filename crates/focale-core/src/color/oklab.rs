//! Oklab perceptual colour space.
//!
//! Björn Ottosson, "A perceptual color space for image processing" (2020),
//! <https://bottosson.github.io/posts/oklab/>. Implemented from CIE XYZ
//! (D65, Y = 1) rather than linear sRGB so it composes with the Rec.2020
//! working space: `XYZ → LMS (M₁) → cube root → Oklab (M₂)`.
//!
//! The matrix constants are Ottosson's published `f64` values rounded to
//! `f32`; the inverses are derived in `f64` and stored precomputed so
//! runtime maths is fixed.

use super::matrix::Mat3;

/// XYZ (D65, Y = 1) → LMS cone response (Ottosson's M₁).
pub const OKLAB_M1: Mat3 = Mat3([
    [0.818933, 0.36186674, -0.12885971],
    [0.032984544, 0.9293119, 0.03614564],
    [0.0482003, 0.26436627, 0.6338517],
]);

/// Non-linear L′M′S′ (cube roots of LMS) → Oklab (Ottosson's M₂).
pub const OKLAB_M2: Mat3 = Mat3([
    [0.21045426, 0.7936178, -0.004072047],
    [1.9779985, -2.4285922, 0.4505937],
    [0.025904037, 0.78277177, -0.80867577],
]);

/// LMS → XYZ: inverse of [`OKLAB_M1`], derived in `f64`.
pub const OKLAB_M1_INV: Mat3 = Mat3([
    [1.2270138, -0.5578, 0.28125614],
    [-0.04058018, 1.1122569, -0.07167668],
    [-0.07638128, -0.42148197, 1.5861632],
]);

/// Oklab → L′M′S′: inverse of [`OKLAB_M2`], derived in `f64`.
pub const OKLAB_M2_INV: Mat3 = Mat3([
    [1.0, 0.39633778, 0.21580376],
    [1.0, -0.105561346, -0.06385417],
    [1.0, -0.089484185, -1.2914855],
]);

/// CIE XYZ (D65, Y = 1) → Oklab `[L, a, b]`.
///
/// `cbrt` preserves sign, so slightly negative LMS values (extreme
/// out-of-gamut colours) are handled without NaN.
pub fn xyz_to_oklab(xyz: [f32; 3]) -> [f32; 3] {
    let lms = OKLAB_M1.mul_vec(xyz);
    OKLAB_M2.mul_vec([lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()])
}

/// Oklab `[L, a, b]` → CIE XYZ (D65, Y = 1). Inverse of [`xyz_to_oklab`].
pub fn oklab_to_xyz(lab: [f32; 3]) -> [f32; 3] {
    let p = OKLAB_M2_INV.mul_vec(lab);
    OKLAB_M1_INV.mul_vec([p[0] * p[0] * p[0], p[1] * p[1] * p[1], p[2] * p[2] * p[2]])
}

/// Oklab `[L, a, b]` → Oklch `[L, C, h]` with hue `h` in radians in
/// (−π, π] (`atan2` convention; 0 = the +a axis).
pub fn oklab_to_oklch(lab: [f32; 3]) -> [f32; 3] {
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    [lab[0], chroma, lab[2].atan2(lab[1])]
}

/// Oklch `[L, C, h]` (hue in radians) → Oklab `[L, a, b]`. Inverse of
/// [`oklab_to_oklch`].
pub fn oklch_to_oklab(lch: [f32; 3]) -> [f32; 3] {
    [lch[0], lch[1] * lch[2].cos(), lch[1] * lch[2].sin()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::primaries::SRGB_TO_XYZ;
    use crate::color::testutil::{assert_close, assert_mat_close, assert_vec_close};

    #[test]
    fn white_is_achromatic_with_unit_lightness() {
        let lab = xyz_to_oklab(SRGB_TO_XYZ.mul_vec([1.0, 1.0, 1.0]));
        assert_close(lab[0], 1.0, 1e-3);
        assert_close(lab[1], 0.0, 1e-3);
        assert_close(lab[2], 0.0, 1e-3);
    }

    #[test]
    fn matches_ottosson_reference_table() {
        // Reference values from the Oklab post (rounded to 3 decimals there).
        assert_vec_close(xyz_to_oklab([1.0, 0.0, 0.0]), [0.450, 1.236, -0.019], 1e-3);
        assert_vec_close(xyz_to_oklab([0.0, 1.0, 0.0]), [0.922, -0.671, 0.263], 1e-3);
        assert_vec_close(xyz_to_oklab([0.0, 0.0, 1.0]), [0.153, -1.415, -0.449], 1e-3);
    }

    #[test]
    fn xyz_round_trip() {
        let samples = [
            [0.9505, 1.0, 1.089],
            [0.4124, 0.2126, 0.0193],
            [0.3576, 0.7152, 0.1192],
            [0.1805, 0.0722, 0.9505],
            [0.2, 0.3, 0.4],
        ];
        for xyz in samples {
            assert_vec_close(oklab_to_xyz(xyz_to_oklab(xyz)), xyz, 1e-4);
        }
    }

    #[test]
    fn inverse_constants_match() {
        assert_mat_close(OKLAB_M1 * OKLAB_M1_INV, Mat3::IDENTITY, 1e-5);
        assert_mat_close(OKLAB_M2 * OKLAB_M2_INV, Mat3::IDENTITY, 1e-5);
    }

    #[test]
    fn oklch_round_trip_and_axes() {
        let lab = [0.7, 0.1, -0.05];
        assert_vec_close(oklch_to_oklab(oklab_to_oklch(lab)), lab, 1e-6);

        let lch_a = oklab_to_oklch([0.5, 0.2, 0.0]);
        assert_close(lch_a[1], 0.2, 1e-6);
        assert_close(lch_a[2], 0.0, 1e-6);

        let lch_b = oklab_to_oklch([0.5, 0.0, 0.2]);
        assert_close(lch_b[1], 0.2, 1e-6);
        assert_close(lch_b[2], std::f32::consts::FRAC_PI_2, 1e-6);
    }
}
