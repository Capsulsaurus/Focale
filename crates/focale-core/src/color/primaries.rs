//! Colour gamuts, their primaries, and RGB↔XYZ matrices (all D65).
//!
//! Matrices are derived from the published CIE 1931 xy chromaticities per
//! SMPTE RP 177 (primaries scaled so the white point maps to XYZ with
//! Y = 1) and stored as precomputed `f32` constants so runtime maths never
//! depends on derivation code. A test re-derives every constant in `f64`
//! and asserts closeness.
//!
//! Sources for the chromaticities:
//! - sRGB / Rec.709: IEC 61966-2-1 / ITU-R BT.709-6
//! - Display P3: SMPTE EG 432-1 (DCI-P3 primaries) with a D65 white
//! - Adobe RGB (1998): Adobe RGB (1998) Color Image Encoding, §4.3
//! - Rec.2020: ITU-R BT.2020-2

use std::fmt;

use serde::{Deserialize, Serialize};

use super::matrix::Mat3;

/// A colour gamut selectable as a rendering or export target.
///
/// The pipeline working space is linear Rec.2020 (docs/architecture.md §3);
/// the other gamuts are display/export targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Gamut {
    /// sRGB / Rec.709 primaries, D65 white (IEC 61966-2-1).
    #[default]
    Srgb,
    /// Display P3: DCI-P3 primaries, D65 white, sRGB transfer.
    DisplayP3,
    /// Adobe RGB (1998).
    AdobeRgb,
    /// ITU-R BT.2020 (the pipeline working space).
    Rec2020,
}

impl Gamut {
    /// All gamuts, in UI presentation order.
    pub const ALL: [Gamut; 4] = [
        Gamut::Srgb,
        Gamut::DisplayP3,
        Gamut::AdobeRgb,
        Gamut::Rec2020,
    ];

    /// Human-readable name for UI display (e.g. the status bar gamut key).
    pub fn display_name(self) -> &'static str {
        match self {
            Gamut::Srgb => "sRGB",
            Gamut::DisplayP3 => "Display P3",
            Gamut::AdobeRgb => "Adobe RGB",
            Gamut::Rec2020 => "Rec. 2020",
        }
    }

    /// CIE 1931 xy chromaticities of the R, G, B primaries.
    ///
    /// `f64` because these are derivation inputs, not pixel maths.
    pub fn primaries(self) -> [[f64; 2]; 3] {
        match self {
            Gamut::Srgb => SRGB_PRIMARIES,
            Gamut::DisplayP3 => DISPLAY_P3_PRIMARIES,
            Gamut::AdobeRgb => ADOBE_RGB_PRIMARIES,
            Gamut::Rec2020 => REC2020_PRIMARIES,
        }
    }

    /// Linear RGB in this gamut → CIE XYZ (D65, Y = 1 at white).
    pub fn rgb_to_xyz(self) -> Mat3 {
        match self {
            Gamut::Srgb => SRGB_TO_XYZ,
            Gamut::DisplayP3 => DISPLAY_P3_TO_XYZ,
            Gamut::AdobeRgb => ADOBE_RGB_TO_XYZ,
            Gamut::Rec2020 => REC2020_TO_XYZ,
        }
    }

    /// CIE XYZ (D65) → linear RGB in this gamut.
    pub fn xyz_to_rgb(self) -> Mat3 {
        match self {
            Gamut::Srgb => XYZ_TO_SRGB,
            Gamut::DisplayP3 => XYZ_TO_DISPLAY_P3,
            Gamut::AdobeRgb => XYZ_TO_ADOBE_RGB,
            Gamut::Rec2020 => XYZ_TO_REC2020,
        }
    }
}

impl fmt::Display for Gamut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// sRGB / Rec.709 primaries: R, G, B rows as CIE xy.
pub const SRGB_PRIMARIES: [[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
/// Display P3 primaries: R, G, B rows as CIE xy.
pub const DISPLAY_P3_PRIMARIES: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];
/// Adobe RGB (1998) primaries: R, G, B rows as CIE xy.
pub const ADOBE_RGB_PRIMARIES: [[f64; 2]; 3] = [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06]];
/// Rec.2020 primaries: R, G, B rows as CIE xy.
pub const REC2020_PRIMARIES: [[f64; 2]; 3] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];

/// Linear sRGB → XYZ (D65).
pub const SRGB_TO_XYZ: Mat3 = Mat3([
    [0.4123908, 0.35758433, 0.1804808],
    [0.212639, 0.71516865, 0.07219232],
    [0.019330818, 0.11919478, 0.95053214],
]);

/// XYZ (D65) → linear sRGB.
pub const XYZ_TO_SRGB: Mat3 = Mat3([
    [3.24097, -1.5373832, -0.49861076],
    [-0.96924365, 1.8759675, 0.04155506],
    [0.05563008, -0.20397696, 1.0569715],
]);

/// Linear Display P3 → XYZ (D65).
pub const DISPLAY_P3_TO_XYZ: Mat3 = Mat3([
    [0.48657095, 0.2656677, 0.19821729],
    [0.22897457, 0.69173855, 0.07928691],
    [0.0, 0.04511338, 1.0439444],
]);

/// XYZ (D65) → linear Display P3.
pub const XYZ_TO_DISPLAY_P3: Mat3 = Mat3([
    [2.493497, -0.9313836, -0.4027108],
    [-0.829489, 1.7626641, 0.023624687],
    [0.03584583, -0.07617239, 0.9568845],
]);

/// Linear Adobe RGB (1998) → XYZ (D65).
pub const ADOBE_RGB_TO_XYZ: Mat3 = Mat3([
    [0.57666904, 0.18555824, 0.18822865],
    [0.29734498, 0.62736356, 0.075291455],
    [0.027031362, 0.07068885, 0.99133754],
]);

/// XYZ (D65) → linear Adobe RGB (1998).
pub const XYZ_TO_ADOBE_RGB: Mat3 = Mat3([
    [2.0415878, -0.565007, -0.34473136],
    [-0.96924365, 1.8759675, 0.04155506],
    [0.01344428, -0.11836239, 1.015175],
]);

/// Linear Rec.2020 (the working space) → XYZ (D65).
pub const REC2020_TO_XYZ: Mat3 = Mat3([
    [0.63695806, 0.1446169, 0.16888097],
    [0.2627002, 0.67799807, 0.059301715],
    [0.0, 0.028072692, 1.0609851],
]);

/// XYZ (D65) → linear Rec.2020 (the working space).
pub const XYZ_TO_REC2020: Mat3 = Mat3([
    [1.7166512, -0.35567078, -0.2533663],
    [-0.6666843, 1.6164812, 0.015768547],
    [0.017639857, -0.042770613, 0.94210315],
]);

/// Linear Rec.2020 → linear sRGB (composite, derived in `f64`).
pub const REC2020_TO_SRGB: Mat3 = Mat3([
    [1.660491, -0.5876411, -0.07284986],
    [-0.12455048, 1.1328999, -0.008349422],
    [-0.018150764, -0.1005789, 1.1187297],
]);

/// Linear sRGB → linear Rec.2020 (composite, derived in `f64`).
pub const SRGB_TO_REC2020: Mat3 = Mat3([
    [0.6274039, 0.32928303, 0.043313067],
    [0.06909729, 0.9195404, 0.011362315],
    [0.01639144, 0.088013306, 0.89559525],
]);

/// Linear Rec.2020 → linear Display P3 (composite, derived in `f64`).
pub const REC2020_TO_DISPLAY_P3: Mat3 = Mat3([
    [1.3435782, -0.28217968, -0.06139858],
    [-0.065297455, 1.0757879, -0.010490463],
    [0.0028217873, -0.019598495, 1.0167767],
]);

/// Linear Display P3 → linear Rec.2020 (composite, derived in `f64`).
pub const DISPLAY_P3_TO_REC2020: Mat3 = Mat3([
    [0.75383306, 0.19859737, 0.047569595],
    [0.04574385, 0.9417772, 0.012478931],
    [-0.0012103403, 0.017601717, 0.9836086],
]);

/// Linear Rec.2020 → linear Adobe RGB (composite, derived in `f64`).
pub const REC2020_TO_ADOBE_RGB: Mat3 = Mat3([
    [1.1519784, -0.09750306, -0.05447534],
    [-0.12455048, 1.1328999, -0.008349422],
    [-0.022530382, -0.04980651, 1.0723369],
]);

/// Linear Adobe RGB → linear Rec.2020 (composite, derived in `f64`).
pub const ADOBE_RGB_TO_REC2020: Mat3 = Mat3([
    [0.8773338, 0.077493705, 0.045172453],
    [0.09662259, 0.8915273, 0.011850088],
    [0.022921063, 0.043036684, 0.9340423],
]);

/// Linear Rec.2020 → linear `target` RGB (both D65; identity for Rec.2020).
pub fn rec2020_to_gamut(target: Gamut) -> Mat3 {
    match target {
        Gamut::Srgb => REC2020_TO_SRGB,
        Gamut::DisplayP3 => REC2020_TO_DISPLAY_P3,
        Gamut::AdobeRgb => REC2020_TO_ADOBE_RGB,
        Gamut::Rec2020 => Mat3::IDENTITY,
    }
}

/// Linear `source` RGB → linear Rec.2020 (both D65; identity for Rec.2020).
pub fn gamut_to_rec2020(source: Gamut) -> Mat3 {
    match source {
        Gamut::Srgb => SRGB_TO_REC2020,
        Gamut::DisplayP3 => DISPLAY_P3_TO_REC2020,
        Gamut::AdobeRgb => ADOBE_RGB_TO_REC2020,
        Gamut::Rec2020 => Mat3::IDENTITY,
    }
}

/// Rec.2020 luminance coefficients: the Y row of [`REC2020_TO_XYZ`]
/// (matches the coefficients published in ITU-R BT.2020-2).
pub const REC2020_LUMINANCE: [f32; 3] = [0.2627002, 0.67799807, 0.059301715];

/// Relative luminance (CIE Y, 1.0 at diffuse white) of a linear Rec.2020
/// pixel.
pub fn luminance_rec2020(rgb: [f32; 3]) -> f32 {
    REC2020_LUMINANCE[0] * rgb[0] + REC2020_LUMINANCE[1] * rgb[1] + REC2020_LUMINANCE[2] * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::adapt::{ILLUMINANT_D65, xy_to_xyz};
    use crate::color::matrix::{Mat3F64, invert3_f64, mul_vec3_f64, mul3_f64};
    use crate::color::testutil::{assert_close, assert_mat_close, assert_vec_close};

    /// Re-derives an RGB→XYZ matrix in `f64` per SMPTE RP 177.
    fn derive_rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 2]) -> Mat3F64 {
        let mut p = [[0.0_f64; 3]; 3];
        for (j, &xy) in primaries.iter().enumerate() {
            let xyz = xy_to_xyz(xy);
            for (i, row) in p.iter_mut().enumerate() {
                row[j] = xyz[i];
            }
        }
        let s = mul_vec3_f64(
            &invert3_f64(&p).expect("primaries are independent"),
            xy_to_xyz(white),
        );
        let mut m = [[0.0_f64; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, entry) in row.iter_mut().enumerate() {
                *entry = p[i][j] * s[j];
            }
        }
        m
    }

    fn assert_matches_f64(actual: Mat3, expected: &Mat3F64, tol: f64) {
        for (row_a, row_e) in actual.0.iter().zip(expected.iter()) {
            for (a, e) in row_a.iter().zip(row_e.iter()) {
                assert!(
                    (f64::from(*a) - e).abs() <= tol,
                    "entry {a} differs from f64 derivation {e}"
                );
            }
        }
    }

    #[test]
    fn consts_match_f64_derivation() {
        for gamut in Gamut::ALL {
            let fwd = derive_rgb_to_xyz(gamut.primaries(), ILLUMINANT_D65);
            let inv = invert3_f64(&fwd).expect("derived matrix is invertible");
            assert_matches_f64(gamut.rgb_to_xyz(), &fwd, 1e-6);
            assert_matches_f64(gamut.xyz_to_rgb(), &inv, 1e-6);
        }
    }

    #[test]
    fn composites_match_f64_derivation() {
        let rec2020 = derive_rgb_to_xyz(REC2020_PRIMARIES, ILLUMINANT_D65);
        let cases = [
            (Gamut::Srgb, REC2020_TO_SRGB, SRGB_TO_REC2020),
            (
                Gamut::DisplayP3,
                REC2020_TO_DISPLAY_P3,
                DISPLAY_P3_TO_REC2020,
            ),
            (Gamut::AdobeRgb, REC2020_TO_ADOBE_RGB, ADOBE_RGB_TO_REC2020),
        ];
        for (gamut, from_2020, to_2020) in cases {
            let target = derive_rgb_to_xyz(gamut.primaries(), ILLUMINANT_D65);
            let target_inv = invert3_f64(&target).expect("invertible");
            let rec2020_inv = invert3_f64(&rec2020).expect("invertible");
            assert_matches_f64(from_2020, &mul3_f64(&target_inv, &rec2020), 1e-6);
            assert_matches_f64(to_2020, &mul3_f64(&rec2020_inv, &target), 1e-6);
        }
    }

    #[test]
    fn forward_inverse_pairs_are_identity() {
        for gamut in Gamut::ALL {
            assert_mat_close(
                gamut.rgb_to_xyz() * gamut.xyz_to_rgb(),
                Mat3::IDENTITY,
                1e-5,
            );
        }
    }

    #[test]
    fn rec2020_xyz_round_trip() {
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.25, 0.5, 0.75],
        ] {
            let back = XYZ_TO_REC2020.mul_vec(REC2020_TO_XYZ.mul_vec(rgb));
            assert_vec_close(back, rgb, 1e-6);
        }
    }

    #[test]
    fn rec2020_gamut_round_trip() {
        for gamut in Gamut::ALL {
            let round = rec2020_to_gamut(gamut) * gamut_to_rec2020(gamut);
            assert_mat_close(round, Mat3::IDENTITY, 1e-5);
        }
    }

    #[test]
    fn srgb_white_maps_to_d65_xyz() {
        let xyz = SRGB_TO_XYZ.mul_vec([1.0, 1.0, 1.0]);
        assert_vec_close(xyz, [0.9505, 1.0, 1.089], 1e-3);
    }

    #[test]
    fn srgb_matrix_matches_published_values() {
        // IEC 61966-2-1 rounded reference values.
        let published = Mat3([
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ]);
        assert_mat_close(SRGB_TO_XYZ, published, 1e-3);
        let published_inv = Mat3([
            [3.2406, -1.5372, -0.4986],
            [-0.9689, 1.8758, 0.0415],
            [0.0557, -0.2040, 1.0570],
        ]);
        assert_mat_close(XYZ_TO_SRGB, published_inv, 1e-3);
    }

    #[test]
    fn white_chromaticity_is_d65_for_all_gamuts() {
        for gamut in Gamut::ALL {
            let [x, y, z] = gamut.rgb_to_xyz().mul_vec([1.0, 1.0, 1.0]);
            let sum = x + y + z;
            assert_close(x / sum, 0.3127, 1e-4);
            assert_close(y / sum, 0.3290, 1e-4);
        }
    }

    #[test]
    fn luminance_matches_matrix_row() {
        assert_eq!(REC2020_LUMINANCE, REC2020_TO_XYZ.0[1]);
        assert_close(luminance_rec2020([1.0, 1.0, 1.0]), 1.0, 1e-5);
        assert_close(luminance_rec2020([0.0, 0.0, 0.0]), 0.0, 0.0);
    }

    #[test]
    fn display_names() {
        assert_eq!(Gamut::Srgb.display_name(), "sRGB");
        assert_eq!(Gamut::DisplayP3.display_name(), "Display P3");
        assert_eq!(Gamut::AdobeRgb.display_name(), "Adobe RGB");
        assert_eq!(Gamut::Rec2020.display_name(), "Rec. 2020");
        assert_eq!(Gamut::Rec2020.to_string(), "Rec. 2020");
        assert_eq!(Gamut::default(), Gamut::Srgb);
    }
}
