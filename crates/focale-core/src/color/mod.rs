//! Colour science: primaries, transfer functions, chromatic adaptation,
//! Oklab, gamut mapping, HDR tone mapping (docs/subsystems/color.md).
//!
//! Everything here runs on the deterministic export path: pixel maths is
//! `f32` with fixed expression order (Rust enables no fast-math or FMA
//! contraction), matrices are precomputed constants (tests re-derive each
//! one in `f64` and assert closeness), and iterative operators use fixed
//! iteration counts. The pipeline working space is linear Rec.2020 (D65,
//! unbounded); this module provides the transforms in and out of it, and
//! the pipeline-versioned gamut-mapping and tone-mapping operators used at
//! export.
//!
//! A soft-proofing transform will slot in between the working image and the
//! display transform post-v1 (docs/subsystems/color.md); nothing here assumes the working →
//! display conversion is a single step.
//!
//! # Transcendentals
//!
//! All transcendental functions (`powf`, `cbrt`, `ln`, `exp`, `atan2`) go
//! through [`crate::math`], which wraps the pure-Rust `libm` crate: platform
//! maths libraries differ across libc versions (a glibc 2.39 vs 2.42
//! divergence was caught by the regression golden), so the export path
//! never calls them. The determinism CI matrix (docs/verification.md)
//! guards this.

pub mod adapt;
pub mod gamut_map;
pub mod matrix;
pub mod oklab;
pub mod primaries;
pub mod tonemap;
pub mod transfer;

pub use adapt::{ILLUMINANT_A, ILLUMINANT_D50, ILLUMINANT_D65, bradford_adaptation, xy_to_xyz};
pub use gamut_map::map_to_gamut;
pub use matrix::Mat3;
pub use oklab::{oklab_to_oklch, oklab_to_xyz, oklch_to_oklab, xyz_to_oklab};
pub use primaries::{
    Gamut, REC2020_LUMINANCE, REC2020_TO_XYZ, XYZ_TO_REC2020, gamut_to_rec2020, luminance_rec2020,
    rec2020_to_gamut,
};
pub use tonemap::{REINHARD_WHITE_DEFAULT, tonemap_reinhard_extended};
pub use transfer::{
    adobe_rgb_decode, adobe_rgb_encode, hlg_oetf, hlg_oetf_inverse, pq_decode, pq_decode_sdr,
    pq_encode, pq_encode_sdr, srgb_decode, srgb_encode,
};

#[cfg(test)]
pub(crate) mod testutil {
    use super::matrix::Mat3;

    /// Asserts `|actual − expected| ≤ tol`.
    pub(crate) fn assert_close(actual: f32, expected: f32, tol: f32) {
        assert!(
            (actual - expected).abs() <= tol,
            "{actual} differs from {expected} by more than {tol}"
        );
    }

    /// Asserts component-wise closeness of two vectors.
    pub(crate) fn assert_vec_close(actual: [f32; 3], expected: [f32; 3], tol: f32) {
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!(
                (a - e).abs() <= tol,
                "{actual:?} differs from {expected:?} by more than {tol}"
            );
        }
    }

    /// Asserts entry-wise closeness of two matrices.
    pub(crate) fn assert_mat_close(actual: Mat3, expected: Mat3, tol: f32) {
        for (row_a, row_e) in actual.0.iter().zip(expected.0.iter()) {
            for (a, e) in row_a.iter().zip(row_e.iter()) {
                assert!(
                    (a - e).abs() <= tol,
                    "{actual:?} differs from {expected:?} by more than {tol}"
                );
            }
        }
    }
}
