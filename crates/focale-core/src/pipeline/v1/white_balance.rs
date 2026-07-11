//! Pipeline v1 stage 3: white balance and the camera → working-space
//! transform — **frozen**: the formulas documented here define the v1 output
//! forever (PRD §2.2).
//!
//! Converts linear camera-native RGB (stage 1 output) to the linear Rec.2020
//! working space, white-balanced so that the selected scene neutral maps to
//! equal RGB. Two per-image derivations feed one fused per-pixel transform:
//!
//! ```text
//! out = M · (gains ⊙ rgb_camera)
//! ```
//!
//! # 1. White-balance gains (green-normalized)
//!
//! `gains = (n_g/n_r, 1, n_g/n_b)` from a camera-space neutral `n`:
//!
//! - [`WhiteBalanceParams::AsShot`]: `n` is
//!   [`RawMetadata::as_shot_neutral`] (already green-normalized); when the
//!   file carries none, `n = (1, 1, 1)` (unity gains).
//! - [`WhiteBalanceParams::Custom`]: the gains are given directly as
//!   `(red, 1, blue)` — no inversion.
//! - [`WhiteBalanceParams::Temperature`]: the assumed scene white is placed
//!   on the standard locus (below), converted `xy → XYZ (Y = 1)`
//!   ([`xy_to_xyz`]), and mapped to camera space through
//!   [`RawMetadata::xyz_to_camera`] (DNG convention). Without a camera
//!   matrix the camera is treated as linear sRGB (`n = XYZ_TO_SRGB · white`).
//!
//! Any neutral with a non-positive or non-finite component falls back to
//! unity gains (never fails, never guesses).
//!
//! ## Temperature → chromaticity (derived in `f64`)
//!
//! `kelvin` is clamped to `[1667, 25000]`.
//!
//! - `T ≥ 4000 K` — the CIE daylight locus polynomial (CIE 15:2004 §5.1.2):
//!
//!   ```text
//!   4000 ≤ T ≤ 7000:  x = −4.6070e9/T³ + 2.9678e6/T² + 0.09911e3/T + 0.244063
//!   7000 <  T ≤ 25000: x = −2.0064e9/T³ + 1.9018e6/T² + 0.24748e3/T + 0.237040
//!   y = −3.000·x² + 2.870·x − 0.275
//!   ```
//!
//! - `T < 4000 K` — the cubic-spline approximation of the **Planckian**
//!   locus by Kim et al., "Design of Advanced Color: Temperature Control
//!   System for HDTV Applications", J. Korean Phys. Soc. 41(6), 2002 (the
//!   widely used piecewise cubic, valid 1667–4000 K):
//!
//!   ```text
//!   x = −0.2661239e9/T³ − 0.2343589e6/T² + 0.8776956e3/T + 0.179910
//!   1667 ≤ T ≤ 2222: y = −1.1063814x³ − 1.34811020x² + 2.18555832x − 0.20219683
//!   2222 <  T ≤ 4000: y = −0.9549476x³ − 1.37418593x² + 2.09137015x − 0.16748867
//!   ```
//!
//!   The two loci do not meet exactly at 4000 K (the daylight locus sits
//!   slightly above the Planckian); the small step there is pinned v1
//!   behaviour.
//!
//! ## Tint
//!
//! `y ← y − tint · 0.0005`. **Pinned**: positive tint *lowers* `y`, placing
//! the assumed white below the locus on the magenta side (magenta is below
//! the Planckian/daylight locus in CIE 1931 `y`); negative tint raises `y`
//! toward green. One tint unit moves `y` by 0.0005.
//!
//! # 2. Camera → Rec.2020 matrix
//!
//! With a camera matrix `C = xyz_to_camera`:
//!
//! ```text
//! M = XYZ_TO_REC2020 · normalize_rows(C⁻¹)
//! ```
//!
//! where `normalize_rows` scales row `i` of the camera→XYZ inverse by
//! `white_xyz[i] / row_sumᵢ` with `white_xyz = D65 (0.9504559, 1.0,
//! 1.0890578)`. This is the dcraw white-point normalization adapted to XYZ:
//! it forces the white-balanced camera neutral `(1, 1, 1)` to map exactly to
//! D65 white in XYZ — and therefore to `(1, 1, 1)` in Rec.2020, whose white
//! is D65. Derived in `f64`, rounded once to `f32` (per-image scalar work,
//! not pixel maths). If the matrix is missing, singular, or a row sum is not
//! strictly positive and finite, the camera is treated as linear sRGB:
//! `M = SRGB_TO_REC2020`.
//!
//! # 3. Per-pixel application
//!
//! One fused transform per pixel — the three gains then the 3×3 multiply —
//! in `f32` with fixed expression order, parallelized with `rayon` over
//! disjoint rows (`par_chunks_mut` with an exact row stride). Output is
//! unbounded linear Rec.2020.

use rayon::prelude::*;

use crate::color::matrix::{Mat3F64, invert3_f64, mat3_from_f64, mul_vec3_f64};
use crate::color::primaries::{SRGB_TO_REC2020, XYZ_TO_SRGB};
use crate::color::{ILLUMINANT_D65, Mat3, XYZ_TO_REC2020, xy_to_xyz};
use crate::decode::RawMetadata;
use crate::image::ImageRgbF32;
use crate::params::white_balance::WhiteBalanceParams;

/// Applies white balance and converts camera RGB to linear Rec.2020 in
/// place (module docs for the frozen formulas).
pub fn apply(image: &mut ImageRgbF32, wb: &WhiteBalanceParams, meta: &RawMetadata) {
    let gains = wb_gains(wb, meta);
    let m = camera_to_rec2020(meta);
    let stride = image.width() as usize * 3;
    if stride == 0 {
        return;
    }
    image.data_mut().par_chunks_mut(stride).for_each(|row| {
        for px in row.chunks_exact_mut(3) {
            let balanced = [px[0] * gains[0], px[1] * gains[1], px[2] * gains[2]];
            let out = m.mul_vec(balanced);
            px[0] = out[0];
            px[1] = out[1];
            px[2] = out[2];
        }
    });
}

/// Computes the green-normalized per-channel gains (module docs §1).
fn wb_gains(wb: &WhiteBalanceParams, meta: &RawMetadata) -> [f32; 3] {
    match *wb {
        WhiteBalanceParams::AsShot => {
            neutral_to_gains(meta.as_shot_neutral.unwrap_or([1.0, 1.0, 1.0]))
        }
        WhiteBalanceParams::Custom { red, blue } => [red, 1.0, blue],
        WhiteBalanceParams::Temperature { kelvin, tint } => {
            let [x, y] = locus_xy(f64::from(kelvin));
            let y = y - f64::from(tint) * 0.0005;
            let white = xy_to_xyz([x, y]);
            let neutral64 = match meta.xyz_to_camera {
                Some(rows) => mul_vec3_f64(&to_f64(rows), white),
                None => mul_vec3_f64(&to_f64(XYZ_TO_SRGB.0), white),
            };
            neutral_to_gains(neutral64.map(|v| v as f32))
        }
    }
}

/// `gains = (n_g/n_r, 1, n_g/n_b)`; unity gains when any component of the
/// neutral is non-positive or non-finite.
fn neutral_to_gains(n: [f32; 3]) -> [f32; 3] {
    if !n.iter().all(|v| v.is_finite() && *v > 0.0) {
        return [1.0, 1.0, 1.0];
    }
    [n[1] / n[0], 1.0, n[1] / n[2]]
}

/// Assumed-white chromaticity for a correlated colour temperature
/// (module docs §1: CIE daylight locus ≥ 4000 K, Kim et al. Planckian cubic
/// below; kelvin clamped to `[1667, 25000]`). All maths in `f64`.
fn locus_xy(kelvin: f64) -> [f64; 2] {
    let t = kelvin.clamp(1667.0, 25000.0);
    if t >= 4000.0 {
        // CIE daylight locus (CIE 15:2004).
        let x = if t <= 7000.0 {
            -4.6070e9 / (t * t * t) + 2.9678e6 / (t * t) + 0.09911e3 / t + 0.244063
        } else {
            -2.0064e9 / (t * t * t) + 1.9018e6 / (t * t) + 0.24748e3 / t + 0.237040
        };
        let y = -3.000 * x * x + 2.870 * x - 0.275;
        [x, y]
    } else {
        // Kim et al. (2002) Planckian cubic approximation.
        let x = -0.2661239e9 / (t * t * t) - 0.2343589e6 / (t * t) + 0.8776956e3 / t + 0.179910;
        let y = if t <= 2222.0 {
            -1.1063814 * x * x * x - 1.34811020 * x * x + 2.18555832 * x - 0.20219683
        } else {
            -0.9549476 * x * x * x - 1.37418593 * x * x + 2.09137015 * x - 0.16748867
        };
        [x, y]
    }
}

/// Derives the camera → Rec.2020 matrix (module docs §2), falling back to
/// `SRGB_TO_REC2020` when no usable camera matrix exists.
fn camera_to_rec2020(meta: &RawMetadata) -> Mat3 {
    let Some(rows) = meta.xyz_to_camera else {
        return SRGB_TO_REC2020;
    };
    let Some(mut cam_to_xyz) = invert3_f64(&to_f64(rows)) else {
        return SRGB_TO_REC2020;
    };
    let white = xy_to_xyz(ILLUMINANT_D65);
    for (row, w) in cam_to_xyz.iter_mut().zip(white) {
        let sum = row[0] + row[1] + row[2];
        if !(sum.is_finite() && sum > 0.0) {
            return SRGB_TO_REC2020;
        }
        let s = w / sum;
        row[0] *= s;
        row[1] *= s;
        row[2] *= s;
    }
    XYZ_TO_REC2020 * mat3_from_f64(&cam_to_xyz)
}

/// Widens a 3×3 `f32` row matrix to `f64` for derivation maths.
fn to_f64(rows: [[f32; 3]; 3]) -> Mat3F64 {
    rows.map(|r| r.map(f64::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::OpticsMetadata;

    /// Metadata for a hypothetical camera whose native space is exactly
    /// linear sRGB (xyz_to_camera = XYZ→sRGB).
    fn srgb_camera_meta(neutral: Option<[f32; 3]>) -> RawMetadata {
        RawMetadata {
            camera_make: None,
            camera_model: None,
            as_shot_neutral: neutral,
            xyz_to_camera: Some(XYZ_TO_SRGB.0),
            orientation: 1,
            capture_time: None,
            iso: None,
            exposure_time: None,
            f_number: None,
            focal_length: None,
            lens_model: None,
            optics: OpticsMetadata::default(),
        }
    }

    fn no_matrix_meta() -> RawMetadata {
        RawMetadata {
            xyz_to_camera: None,
            ..srgb_camera_meta(None)
        }
    }

    #[test]
    fn as_shot_neutral_maps_to_equal_rgb_at_white() {
        let neutral = [0.5_f32, 1.0, 0.8];
        let meta = srgb_camera_meta(Some(neutral));
        let mut img = ImageRgbF32::from_data(1, 1, neutral.to_vec());
        apply(&mut img, &WhiteBalanceParams::AsShot, &meta);
        let out = img.pixel(0, 0);
        // Gains bring the neutral to (1,1,1); the row-normalized matrix maps
        // (1,1,1) to D65 white = (1,1,1) in Rec.2020.
        for c in out {
            assert!((c - 1.0).abs() < 1e-3, "neutral must map to white: {out:?}");
        }
        assert!((out[0] - out[1]).abs() < 1e-3 && (out[1] - out[2]).abs() < 1e-3);
    }

    #[test]
    fn as_shot_without_neutral_is_unity_gains() {
        let meta = srgb_camera_meta(None);
        assert_eq!(
            wb_gains(&WhiteBalanceParams::AsShot, &meta),
            [1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn srgb_fallback_matches_srgb_to_rec2020() {
        let meta = no_matrix_meta();
        let wb = WhiteBalanceParams::Custom {
            red: 1.5,
            blue: 0.75,
        };
        let mut img = ImageRgbF32::from_data(1, 1, vec![1.0, 0.0, 0.0]);
        apply(&mut img, &wb, &meta);
        // Red camera pixel with gain 1.5 → SRGB_TO_REC2020 · (1.5, 0, 0).
        let expected = SRGB_TO_REC2020.mul_vec([1.5, 0.0, 0.0]);
        assert_eq!(img.pixel(0, 0), expected);
    }

    #[test]
    fn d65_temperature_is_near_no_op_for_srgb_camera() {
        let meta = srgb_camera_meta(None);
        let gains = wb_gains(
            &WhiteBalanceParams::Temperature {
                kelvin: 6504.0,
                tint: 0.0,
            },
            &meta,
        );
        // The daylight locus at 6504 K is very close to D65; an sRGB camera
        // needs ~unity gains there (loose tolerance: locus ≠ exact D65).
        for g in gains {
            assert!(
                (g - 1.0).abs() < 0.02,
                "gains must be near unity: {gains:?}"
            );
        }
    }

    #[test]
    fn higher_kelvin_is_warmer() {
        let meta = srgb_camera_meta(None);
        let g = |k: f32| {
            wb_gains(
                &WhiteBalanceParams::Temperature {
                    kelvin: k,
                    tint: 0.0,
                },
                &meta,
            )
        };
        let cool = g(5000.0);
        let warm = g(8000.0);
        assert!(
            warm[0] / warm[2] > cool[0] / cool[2],
            "higher kelvin must raise r gain relative to b: {warm:?} vs {cool:?}"
        );
        // And monotonic across both loci — starting at 2500 K: below that
        // the assumed white leaves the sRGB test-camera's gamut (negative
        // blue), which triggers the pinned unity-gain fallback.
        let mut prev = f32::NEG_INFINITY;
        for k in [2500.0, 3500.0, 4500.0, 6500.0, 10000.0, 20000.0] {
            let gains = g(k);
            let ratio = gains[0] / gains[2];
            assert!(ratio > prev, "r/b gain ratio must rise with kelvin at {k}");
            prev = ratio;
        }
    }

    #[test]
    fn out_of_gamut_white_falls_back_to_unity_gains() {
        // 1667 K on the Planckian locus is outside the sRGB test-camera's
        // gamut (negative blue channel): pinned fallback is unity gains.
        let meta = srgb_camera_meta(None);
        let gains = wb_gains(
            &WhiteBalanceParams::Temperature {
                kelvin: 1667.0,
                tint: 0.0,
            },
            &meta,
        );
        assert_eq!(gains, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn positive_tint_lowers_assumed_white_y() {
        // Pinned direction: positive tint shifts the assumed white below the
        // locus (magenta side), which lowers the red/blue gains relative to
        // green for an sRGB camera.
        let meta = srgb_camera_meta(None);
        let g = |tint: f32| {
            wb_gains(
                &WhiteBalanceParams::Temperature {
                    kelvin: 6500.0,
                    tint,
                },
                &meta,
            )
        };
        let neutral = g(0.0);
        let magenta = g(50.0);
        assert!(magenta[0] < neutral[0]);
        assert!(magenta[2] < neutral[2]);
        assert_eq!(magenta[1], 1.0);
    }

    #[test]
    fn kelvin_is_clamped_to_pinned_range() {
        assert_eq!(locus_xy(100.0), locus_xy(1667.0));
        assert_eq!(locus_xy(1e9), locus_xy(25000.0));
    }

    #[test]
    fn locus_matches_reference_chromaticities() {
        // D65's defining CCT is ~6504 K; the daylight locus there must land
        // on (0.3127, 0.3290) to ~1e-3.
        let [x, y] = locus_xy(6504.0);
        assert!((x - 0.3127).abs() < 1e-3, "x = {x}");
        assert!((y - 0.3290).abs() < 1e-3, "y = {y}");
        // Planckian branch at 2856 K (illuminant A ≈ (0.4476, 0.4074)).
        let [x, y] = locus_xy(2856.0);
        assert!((x - 0.4476).abs() < 5e-3, "x = {x}");
        assert!((y - 0.4074).abs() < 5e-3, "y = {y}");
    }

    #[test]
    fn degenerate_neutral_falls_back_to_unity() {
        assert_eq!(neutral_to_gains([0.0, 1.0, 1.0]), [1.0, 1.0, 1.0]);
        assert_eq!(neutral_to_gains([0.5, -1.0, 1.0]), [1.0, 1.0, 1.0]);
        assert_eq!(neutral_to_gains([f32::NAN, 1.0, 1.0]), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn singular_camera_matrix_falls_back_to_srgb() {
        let meta = RawMetadata {
            xyz_to_camera: Some([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]),
            ..srgb_camera_meta(None)
        };
        assert_eq!(camera_to_rec2020(&meta), SRGB_TO_REC2020);
    }

    #[test]
    fn camera_matrix_white_lands_on_rec2020_white() {
        // Any invertible camera matrix must, after row normalization, map the
        // balanced neutral (1,1,1) to Rec.2020 white.
        let meta = RawMetadata {
            // A plausible warm-biased camera matrix (arbitrary, invertible).
            xyz_to_camera: Some([[0.9, 0.2, -0.1], [-0.3, 1.1, 0.15], [0.05, -0.2, 0.8]]),
            ..srgb_camera_meta(None)
        };
        let m = camera_to_rec2020(&meta);
        let out = m.mul_vec([1.0, 1.0, 1.0]);
        for c in out {
            assert!((c - 1.0).abs() < 1e-4, "white must be preserved: {out:?}");
        }
    }
}
