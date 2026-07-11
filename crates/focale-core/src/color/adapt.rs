//! Bradford chromatic adaptation and CIE standard illuminants.
//!
//! Camera colour matrices (the DNG `ColorMatrix1/2` convention) are relative
//! to a calibration illuminant, so bringing camera colours into the D65
//! working space requires adapting between white points. We use the Bradford
//! transform (Lam, "Metamerism and Colour Constancy", 1985), as adopted by
//! ICC.1 and described in Lindbloom, "Chromatic Adaptation"
//! (<http://www.brucelindbloom.com/Eqn_ChromAdapt.html>).
//!
//! All derivation maths here is `f64` (scalar, per-image work — not pixel
//! maths) with fixed expression order; results are deterministic.

use super::matrix::{Mat3, Mat3F64, invert3_f64, mul_vec3_f64, mul3_f64};

/// CIE standard illuminant A (tungsten), xy for the 2° observer (CIE 15).
pub const ILLUMINANT_A: [f64; 2] = [0.44757, 0.40745];

/// CIE standard illuminant D50 (horizon light, ICC PCS white), xy for the
/// 2° observer (CIE 15).
pub const ILLUMINANT_D50: [f64; 2] = [0.34567, 0.35850];

/// CIE standard illuminant D65 (noon daylight), the 4-digit xy used by the
/// sRGB / BT.709 / BT.2020 specifications and all matrices in this module.
pub const ILLUMINANT_D65: [f64; 2] = [0.3127, 0.3290];

/// Bradford cone-response matrix (XYZ → RGB-like cone space), from
/// Lam (1985) / ICC.1.
const BRADFORD: Mat3F64 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// Converts a CIE xy chromaticity to XYZ with Y = 1.
pub fn xy_to_xyz(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

/// Bradford chromatic adaptation matrix from `src_white_xy` to
/// `dst_white_xy`.
///
/// The returned matrix maps XYZ colours viewed under the source illuminant
/// to corresponding XYZ colours under the destination illuminant; in
/// particular it maps the source white (Y = 1) exactly onto the destination
/// white. Derived in `f64`, rounded once to `f32`.
pub fn bradford_adaptation(src_white_xy: [f64; 2], dst_white_xy: [f64; 2]) -> Mat3 {
    let src_cone = mul_vec3_f64(&BRADFORD, xy_to_xyz(src_white_xy));
    let dst_cone = mul_vec3_f64(&BRADFORD, xy_to_xyz(dst_white_xy));
    let scale: Mat3F64 = [
        [dst_cone[0] / src_cone[0], 0.0, 0.0],
        [0.0, dst_cone[1] / src_cone[1], 0.0],
        [0.0, 0.0, dst_cone[2] / src_cone[2]],
    ];
    let bradford_inv = invert3_f64(&BRADFORD).expect("Bradford matrix is invertible");
    let adapted = mul3_f64(&bradford_inv, &mul3_f64(&scale, &BRADFORD));
    super::matrix::mat3_from_f64(&adapted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::testutil::{assert_mat_close, assert_vec_close};

    #[test]
    fn same_white_is_identity() {
        let m = bradford_adaptation(ILLUMINANT_D65, ILLUMINANT_D65);
        assert_mat_close(m, Mat3::IDENTITY, 1e-6);
    }

    #[test]
    fn a_to_d65_maps_whites_exactly() {
        let m = bradford_adaptation(ILLUMINANT_A, ILLUMINANT_D65);
        let white_a = xy_to_xyz(ILLUMINANT_A).map(|v| v as f32);
        let white_d65 = xy_to_xyz(ILLUMINANT_D65).map(|v| v as f32);
        assert_vec_close(m.mul_vec(white_a), white_d65, 1e-5);
    }

    #[test]
    fn a_to_d65_matches_f64_reference() {
        // Fixed reference derived in f64 from the constants above; guards
        // against regressions in the derivation.
        let expected = Mat3([
            [0.84468, -0.11793686, 0.39497316],
            [-0.13663386, 1.1041075, 0.12922217],
            [0.0798857, -0.13496444, 3.1933606],
        ]);
        assert_mat_close(
            bradford_adaptation(ILLUMINANT_A, ILLUMINANT_D65),
            expected,
            1e-6,
        );
    }

    #[test]
    fn adaptation_round_trip_is_identity() {
        let fwd = bradford_adaptation(ILLUMINANT_A, ILLUMINANT_D65);
        let back = bradford_adaptation(ILLUMINANT_D65, ILLUMINANT_A);
        assert_mat_close(fwd * back, Mat3::IDENTITY, 1e-4);
        let fwd50 = bradford_adaptation(ILLUMINANT_D50, ILLUMINANT_D65);
        let back50 = bradford_adaptation(ILLUMINANT_D65, ILLUMINANT_D50);
        assert_mat_close(fwd50 * back50, Mat3::IDENTITY, 1e-4);
    }

    #[test]
    fn xy_to_xyz_d65_reference() {
        let [x, y, z] = xy_to_xyz(ILLUMINANT_D65);
        assert!((x - 0.9505).abs() < 1e-4);
        assert!((y - 1.0).abs() < f64::EPSILON);
        assert!((z - 1.0891).abs() < 1e-4);
    }
}
