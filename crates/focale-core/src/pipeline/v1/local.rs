//! Pipeline v1 stage 6: local adjustments — **frozen**: the formulas
//! documented here define the v1 output forever (PRD §2.2).
//!
//! Each enabled [`LocalAdjustment`] is applied in list order:
//!
//! 1. its mask group is rasterized against the **current** image
//!    ([`super::masks::rasterize_group`] — range masks therefore see the
//!    output of every earlier adjustment);
//! 2. a fully adjusted copy of the frame is computed by applying the
//!    adjustment's parameter deltas everywhere (the ops below);
//! 3. the result is a per-pixel linear blend by mask coverage `cov ∈ [0, 1]`:
//!
//!    ```text
//!    out = base · (1 − cov) + adjusted · cov
//!    ```
//!
//!    (pinned in this two-product form so `cov = 0` returns `base` and
//!    `cov = 1` returns `adjusted` bit-exactly).
//!
//! Because adjustments are applied sequentially, their order matters: a
//! later adjustment operates on (and its range masks sample) the blended
//! output of earlier ones.
//!
//! # Parameter application (fixed order)
//!
//! [`LocalParams`] holds deltas — every 0 / identity value is a no-op, and
//! each block below is skipped entirely when neutral:
//!
//! 1. **Tone**: exposure, contrast, highlights, shadows, whites, blacks and
//!    the point curve run the exact stage-4 scalar chain
//!    ([`super::tone::ToneOps`]) with the delta values — luminance-ratio
//!    preserving, identical formulas.
//! 2. **Temperature / tint** — a deliberately simple channel-gain
//!    approximation of white-balance trim in the Rec.2020 working space
//!    (a true WB trim would re-enter camera space, which stage 6 cannot do):
//!
//!    ```text
//!    temperature t: r ·= 2^(t/100 · 0.3),  b ·= 2^(−t/100 · 0.3)
//!    tint ti:       g ·= 2^(−ti/100 · 0.2)
//!    ```
//!
//!    Positive `t` warms (red up, blue down); positive `ti` shifts toward
//!    magenta (green down).
//! 3. **Colour** (in Oklab, like stage 5; same skip rules — non-finite
//!    pixels skipped, Oklab `L ≤ 0` untouched):
//!    - `tint_wheel`: a stage-5 grading wheel with **weight 1 everywhere**:
//!      `a += Δa`, `b += Δb`, `L ·= 2^l_exp`
//!      ([`super::color_grade::wheel_offsets`]);
//!    - vibrance, then saturation, exactly as stage 5
//!      ([`super::color_grade::vibrance_factor`],
//!      [`super::color_grade::saturation_factor`]).
//!
//! # Determinism
//!
//! `f32` pixel maths with fixed expression order; `rayon` only over disjoint
//! rows (`par_chunks_mut`/`par_chunks` with exact row strides); mask
//! rasterization is itself deterministic (see [`super::masks`]). Note this
//! stage takes no preview `scale`: none of its parameters is
//! pixel-dimensioned (mask feather is stored as a fraction of the long
//! image edge and resolves inside the rasterizer).

use rayon::prelude::*;

use super::color_grade::{
    oklab_to_rec2020, rec2020_to_oklab, saturation_factor, vibrance_factor, wheel_offsets,
};
use super::masks::{MaskContext, rasterize_group};
use super::tone::{ToneOps, apply_tone_ops};
use crate::image::{ImageGrayF32, ImageRgbF32};
use crate::params::local::{LocalAdjustment, LocalParams};

/// Applies every enabled local adjustment in order (module docs): rasterize
/// the mask against the current image, apply the deltas frame-wide, blend
/// by coverage.
pub fn apply(image: &mut ImageRgbF32, adjustments: &[LocalAdjustment]) {
    for adjustment in adjustments {
        if !adjustment.enabled {
            continue;
        }
        let coverage = rasterize_group(
            &adjustment.mask,
            &MaskContext {
                width: image.width(),
                height: image.height(),
                image,
            },
        );
        apply_with_coverage(image, &adjustment.adjustments, &coverage);
    }
}

/// Applies one adjustment through an already-rasterized coverage plane:
/// `out = base·(1 − cov) + adjusted·cov` per pixel (module docs).
///
/// Split from [`apply`] so the blend semantics are testable independently
/// of mask rasterization.
///
/// # Panics
/// If the coverage plane's dimensions differ from the image's.
pub(crate) fn apply_with_coverage(
    image: &mut ImageRgbF32,
    params: &LocalParams,
    coverage: &ImageGrayF32,
) {
    assert_eq!(
        (coverage.width(), coverage.height()),
        (image.width(), image.height()),
        "coverage plane must match image dimensions"
    );
    let width = image.width() as usize;
    let stride = width * 3;
    if stride == 0 {
        return;
    }

    let mut adjusted = image.clone();
    apply_local_params(&mut adjusted, params);

    image
        .data_mut()
        .par_chunks_mut(stride)
        .zip(adjusted.data().par_chunks(stride))
        .zip(coverage.data().par_chunks(width))
        .for_each(|((base_row, adj_row), cov_row)| {
            for ((base, adj), &cov) in base_row
                .chunks_exact_mut(3)
                .zip(adj_row.chunks_exact(3))
                .zip(cov_row.iter())
            {
                let keep = 1.0 - cov;
                base[0] = base[0] * keep + adj[0] * cov;
                base[1] = base[1] * keep + adj[1] * cov;
                base[2] = base[2] * keep + adj[2] * cov;
            }
        });
}

/// Applies the parameter deltas to the whole frame in the pinned order
/// (module docs): tone chain → temperature/tint gains → Oklab colour ops.
fn apply_local_params(image: &mut ImageRgbF32, params: &LocalParams) {
    // 1. Tone chain (stage-4 formulas with the delta values).
    let tone_active = params.exposure != 0.0
        || params.contrast != 0.0
        || params.highlights != 0.0
        || params.shadows != 0.0
        || params.whites != 0.0
        || params.blacks != 0.0
        || !params.curve.is_identity();
    if tone_active {
        let ops = ToneOps::new(
            params.exposure,
            params.contrast,
            params.highlights,
            params.shadows,
            params.whites,
            params.blacks,
            &params.curve,
        );
        apply_tone_ops(image, &ops);
    }

    let stride = image.width() as usize * 3;
    if stride == 0 {
        return;
    }

    // 2. Temperature / tint channel gains.
    if params.temperature != 0.0 || params.tint != 0.0 {
        let gain_r = crate::math::exp2(params.temperature / 100.0 * 0.3);
        let gain_b = crate::math::exp2(-params.temperature / 100.0 * 0.3);
        let gain_g = crate::math::exp2(-params.tint / 100.0 * 0.2);
        image.data_mut().par_chunks_mut(stride).for_each(|row| {
            for px in row.chunks_exact_mut(3) {
                px[0] *= gain_r;
                px[1] *= gain_g;
                px[2] *= gain_b;
            }
        });
    }

    // 3. Oklab colour ops: tint wheel (weight 1) → vibrance → saturation.
    let wheel = wheel_offsets(&params.tint_wheel);
    let color_active = wheel.is_active() || params.vibrance != 0.0 || params.saturation != 0.0;
    if color_active {
        let wheel_l_gain = crate::math::exp2(wheel.l_exp);
        let vibrance = params.vibrance;
        let saturation = params.saturation;
        image.data_mut().par_chunks_mut(stride).for_each(|row| {
            for px in row.chunks_exact_mut(3) {
                let rgb = [px[0], px[1], px[2]];
                if !(rgb[0].is_finite() && rgb[1].is_finite() && rgb[2].is_finite()) {
                    continue;
                }
                let lab = rec2020_to_oklab(rgb);
                if lab[0].is_nan() || lab[0] <= 0.0 {
                    continue;
                }
                let [mut l, mut a, mut b] = lab;
                a += wheel.da;
                b += wheel.db;
                l *= wheel_l_gain;
                if vibrance != 0.0 {
                    let c = (a * a + b * b).sqrt();
                    let f = vibrance_factor(c, vibrance);
                    a *= f;
                    b *= f;
                }
                if saturation != 0.0 {
                    let f = saturation_factor(saturation);
                    a *= f;
                    b *= f;
                }
                let out = oklab_to_rec2020([l, a, b]);
                px[0] = out[0];
                px[1] = out[1];
                px[2] = out[2];
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masks::MaskGroup;
    use crate::params::tone::ToneParams;

    fn probe_image() -> ImageRgbF32 {
        let pixels: Vec<[f32; 3]> = vec![
            [0.02, 0.03, 0.01],
            [0.1, 0.05, 0.2],
            [0.18, 0.18, 0.18],
            [0.5, 0.4, 0.3],
            [0.9, 0.95, 1.0],
            [1.5, 1.2, 0.8],
        ];
        ImageRgbF32::from_data(3, 2, pixels.into_iter().flatten().collect())
    }

    fn flat_coverage(width: u32, height: u32, value: f32) -> ImageGrayF32 {
        ImageGrayF32::from_data(width, height, vec![value; (width * height) as usize])
    }

    fn exposure_params(ev: f32) -> LocalParams {
        LocalParams {
            exposure: ev,
            ..LocalParams::default()
        }
    }

    #[test]
    fn zero_coverage_leaves_image_untouched() {
        let mut img = probe_image();
        let before = img.data().to_vec();
        let cov = flat_coverage(img.width(), img.height(), 0.0);
        apply_with_coverage(&mut img, &exposure_params(2.0), &cov);
        assert_eq!(img.data(), before.as_slice());
    }

    #[test]
    fn full_coverage_exposure_matches_global_tone_stage() {
        let mut local_img = probe_image();
        let cov = flat_coverage(local_img.width(), local_img.height(), 1.0);
        apply_with_coverage(&mut local_img, &exposure_params(1.0), &cov);

        let mut tone_img = probe_image();
        super::super::tone::apply(
            &mut tone_img,
            &ToneParams {
                exposure: 1.0,
                ..ToneParams::default()
            },
        );

        assert_eq!(local_img.data(), tone_img.data());
    }

    #[test]
    fn half_coverage_blends_linearly() {
        let params = exposure_params(1.0);

        let mut full = probe_image();
        let cov1 = flat_coverage(full.width(), full.height(), 1.0);
        apply_with_coverage(&mut full, &params, &cov1);

        let mut half = probe_image();
        let cov_half = flat_coverage(half.width(), half.height(), 0.5);
        apply_with_coverage(&mut half, &params, &cov_half);

        let base = probe_image();
        for ((h, b), f) in half
            .data()
            .iter()
            .zip(base.data().iter())
            .zip(full.data().iter())
        {
            // Exactly the pinned blend expression at cov = 0.5.
            assert_eq!(*h, b * 0.5 + f * 0.5);
        }
    }

    #[test]
    fn adjustment_order_matters() {
        // Exposure then contrast ≠ contrast then exposure (the contrast
        // pivot at 0.18 sees different luminances).
        let exposure = exposure_params(1.0);
        let contrast = LocalParams {
            contrast: 60.0,
            ..LocalParams::default()
        };

        let mut ab = ImageRgbF32::from_data(1, 1, vec![0.1, 0.1, 0.1]);
        let cov = flat_coverage(1, 1, 1.0);
        apply_with_coverage(&mut ab, &exposure, &cov);
        apply_with_coverage(&mut ab, &contrast, &cov);

        let mut ba = ImageRgbF32::from_data(1, 1, vec![0.1, 0.1, 0.1]);
        apply_with_coverage(&mut ba, &contrast, &cov);
        apply_with_coverage(&mut ba, &exposure, &cov);

        assert!(
            (ab.pixel(0, 0)[0] - ba.pixel(0, 0)[0]).abs() > 1e-3,
            "order must matter: {} vs {}",
            ab.pixel(0, 0)[0],
            ba.pixel(0, 0)[0]
        );
    }

    #[test]
    fn temperature_warms_and_tint_shifts_magenta() {
        let params = LocalParams {
            temperature: 100.0,
            tint: 100.0,
            ..LocalParams::default()
        };
        let mut img = ImageRgbF32::from_data(1, 1, vec![0.5, 0.5, 0.5]);
        let cov = flat_coverage(1, 1, 1.0);
        apply_with_coverage(&mut img, &params, &cov);
        let [r, g, b] = img.pixel(0, 0);
        assert!(
            (r - 0.5 * crate::math::exp2(0.3_f32)).abs() < 1e-6,
            "r gains 2^0.3"
        );
        assert!(
            (b - 0.5 * crate::math::exp2(-0.3_f32)).abs() < 1e-6,
            "b loses 2^0.3"
        );
        assert!(
            (g - 0.5 * crate::math::exp2(-0.2_f32)).abs() < 1e-6,
            "g loses 2^0.2"
        );
    }

    #[test]
    fn saturation_delta_desaturates_within_mask() {
        let params = LocalParams {
            saturation: -100.0,
            ..LocalParams::default()
        };
        let colorful = [0.5_f32, 0.2, 0.1];
        let mut img = ImageRgbF32::from_data(1, 1, colorful.to_vec());
        let cov = flat_coverage(1, 1, 1.0);
        apply_with_coverage(&mut img, &params, &cov);

        let chroma = |rgb: [f32; 3]| {
            let lab = rec2020_to_oklab(rgb);
            (lab[1] * lab[1] + lab[2] * lab[2]).sqrt()
        };
        let ratio = chroma(img.pixel(0, 0)) / chroma(colorful);
        assert!((ratio - 0.2).abs() < 0.01, "s = −100 scales chroma by 0.2");
    }

    #[test]
    fn tint_wheel_applies_everywhere() {
        // Unlike stage-5 zone wheels, the local tint wheel has weight 1 at
        // every lightness.
        let params = LocalParams {
            tint_wheel: crate::params::color::GradingWheel {
                hue: 0.0,
                saturation: 100.0,
                luminance: 0.0,
            },
            ..LocalParams::default()
        };
        let mut img = ImageRgbF32::from_data(2, 1, vec![0.02, 0.02, 0.02, 0.9, 0.9, 0.9]);
        let cov = flat_coverage(2, 1, 1.0);
        apply_with_coverage(&mut img, &params, &cov);
        for x in 0..2 {
            let a = rec2020_to_oklab(img.pixel(x, 0))[1];
            assert!(
                (a - 0.08).abs() < 1e-3,
                "pixel {x} must get the full +0.08 a offset: {a}"
            );
        }
    }

    #[test]
    fn neutral_params_are_identity_through_full_coverage() {
        // All-zero deltas skip every block: bit-exact pass-through even at
        // full coverage.
        let mut img = probe_image();
        let before = img.data().to_vec();
        let cov = flat_coverage(img.width(), img.height(), 1.0);
        apply_with_coverage(&mut img, &LocalParams::default(), &cov);
        assert_eq!(img.data(), before.as_slice());
    }

    #[test]
    fn disabled_adjustments_are_skipped_by_apply() {
        let mut img = probe_image();
        let before = img.data().to_vec();
        let adjustments = vec![LocalAdjustment {
            enabled: false,
            mask: MaskGroup {
                name: "test".to_string(),
                components: vec![],
            },
            adjustments: exposure_params(3.0),
        }];
        apply(&mut img, &adjustments);
        assert_eq!(img.data(), before.as_slice());

        // And an empty list is trivially a no-op.
        apply(&mut img, &[]);
        assert_eq!(img.data(), before.as_slice());
    }
}
