//! Pipeline v1 stage 5: global colour — **frozen**: the formulas documented
//! here define the v1 output forever (HARD-VER).
//!
//! Works per pixel in Oklab/Oklch derived from the linear Rec.2020 working
//! space: `rec2020 → XYZ → Oklab` ([`crate::color`]), adjustments applied,
//! then converted back. Results may leave the Rec.2020 gamut (negative or
//! above-white components); they are left unbounded — gamut mapping happens
//! at the output transform.
//!
//! Fixed operator order: **HSL bands → grading wheels → vibrance →
//! saturation**.
//!
//! Safety rules (pinned): pixels with any non-finite component are skipped
//! (left untouched); pixels whose Oklab `L ≤ 0` pass through untouched.
//! All other pixels round-trip through Oklab even when every parameter is
//! neutral, so "identity" for this stage is the (near-exact) Oklab
//! round-trip, not a bit-exact pass-through.
//!
//! # HSL bands
//!
//! Eight fixed band centres, pinned as **Oklch hue angles in degrees**
//! (`h = atan2(b, a)`, normalized to `[0, 360)`):
//!
//! ```text
//! red 30, orange 60, yellow 95, green 140, aqua 200, blue 260,
//! purple 305, magenta 345
//! ```
//!
//! Band weight is a raised cosine over ±40° of wrapped angular distance
//! `d`: `w = 0.5·(1 + cos(π·d/40°))` for `d < 40°`, else 0 — scaled by the
//! chroma engagement `e = smoothstep(0, 0.06, C)` so near-neutral pixels
//! are untouched. With per-band sliders in `[−100, 100]`:
//!
//! ```text
//! h += Σᵢ w·e·(hueᵢ/100)·25°
//! C ·= 2^(Σᵢ w·e·satᵢ/100·0.7)      (= Πᵢ 2^(w·e·satᵢ/100·0.7))
//! L ·= 2^(Σᵢ w·e·lumᵢ/100·0.4)
//! ```
//!
//! Band weights are computed from the **original** hue; the accumulated
//! shift is applied once after the band loop (fixed band order red →
//! magenta). The chroma/luminance products are pinned as a single `exp2`
//! of the accumulated exponent sum.
//!
//! # Grading wheels
//!
//! Zone weights come from the (post-HSL) **Oklab L directly** (pinned; no
//! transfer-function encoding), with `bal = balance/100·0.15`:
//!
//! ```text
//! w_s = 1 − smoothstep(0.25 + bal, 0.55 + bal, L)
//! w_h = smoothstep(0.45 + bal, 0.75 + bal, L)
//! w_m = max(0, 1 − w_s − w_h)
//! ```
//!
//! `blending` is ignored in v1 (fixed zone softness — the smoothstep spans
//! above); the parameter is reserved for a future pipeline version.
//!
//! Per wheel, applied sequentially in the fixed order shadows → midtones →
//! highlights (hue in degrees, saturation 0–100, luminance −100..100):
//!
//! ```text
//! a += w · (sat/100) · 0.08 · cos(hue·π/180)
//! b += w · (sat/100) · 0.08 · sin(hue·π/180)
//! L ·= 2^(w · lum/100 · 0.3)
//! ```
//!
//! # Vibrance and saturation
//!
//! Both scale `(a, b)` about the neutral axis; the factors are clamped to
//! ≥ 0 so chroma floors at 0. Vibrance weights toward muted colours using
//! the chroma at its point in the chain (after the wheels):
//!
//! ```text
//! vibrance:   C ·= max(0, 1 + (v/100)·0.6·(1 − clamp(C/0.35, 0, 1)))
//! saturation: C ·= max(0, 1 + (s/100)·0.8)
//! ```
//!
//! applied sequentially (vibrance first, its factor from the pre-vibrance
//! chroma).
//!
//! # Determinism
//!
//! `f32` pixel maths with the fixed expression orders written here; trig
//! and `exp2` resolve per the [`crate::color`] platform-libm caveat.
//! `rayon` parallelism is over disjoint rows only (`par_chunks_mut` with an
//! exact row stride).

use rayon::prelude::*;

use super::tone::smoothstep;
use crate::color::{REC2020_TO_XYZ, XYZ_TO_REC2020, oklab_to_xyz, xyz_to_oklab};
use crate::image::ImageRgbF32;
use crate::params::color::{ColorParams, GradingWheel, HSL_BAND_COUNT, HslBands};

/// The eight HSL band centres as Oklch hue angles in degrees (module docs),
/// index-aligned with [`crate::params::color::HSL_BAND_NAMES`].
pub(crate) const HSL_BAND_CENTERS_DEG: [f32; HSL_BAND_COUNT] =
    [30.0, 60.0, 95.0, 140.0, 200.0, 260.0, 305.0, 345.0];

/// Raised-cosine band half-width in degrees.
const HSL_BAND_WIDTH_DEG: f32 = 40.0;

/// Applies the global colour stage in place (module docs for the frozen
/// formulas). Does nothing when `params.enabled` is false.
pub fn apply(image: &mut ImageRgbF32, params: &ColorParams) {
    if !params.enabled {
        return;
    }

    let hsl_active = hsl_bands_active(&params.hsl);
    let bal = params.grading.balance / 100.0 * 0.15;
    let wheels = [
        wheel_offsets(&params.grading.shadows),
        wheel_offsets(&params.grading.midtones),
        wheel_offsets(&params.grading.highlights),
    ];
    let wheels_active = wheels.iter().any(WheelOffsets::is_active);

    let stride = image.width() as usize * 3;
    if stride == 0 {
        return;
    }
    image.data_mut().par_chunks_mut(stride).for_each(|row| {
        for px in row.chunks_exact_mut(3) {
            let rgb = [px[0], px[1], px[2]];
            if !(rgb[0].is_finite() && rgb[1].is_finite() && rgb[2].is_finite()) {
                continue; // pinned: non-finite pixels are skipped
            }
            let lab = rec2020_to_oklab(rgb);
            if lab[0].is_nan() || lab[0] <= 0.0 {
                continue; // pinned: L ≤ 0 (or NaN) passes through untouched
            }
            let [mut l, mut a, mut b] = lab;

            // 1. HSL bands (Oklch).
            if hsl_active {
                let c = (a * a + b * b).sqrt();
                let e = smoothstep(0.0, 0.06, c);
                let mut h_deg = crate::math::atan2(b, a).to_degrees().rem_euclid(360.0);
                let mut hue_shift = 0.0_f32;
                let mut sat_exp = 0.0_f32;
                let mut lum_exp = 0.0_f32;
                for (i, &center) in HSL_BAND_CENTERS_DEG.iter().enumerate() {
                    let d = (h_deg - center).abs();
                    let d = d.min(360.0 - d);
                    if d < HSL_BAND_WIDTH_DEG {
                        let w = 0.5
                            * (1.0
                                + crate::math::cos(std::f32::consts::PI * d / HSL_BAND_WIDTH_DEG));
                        let we = w * e;
                        hue_shift += we * (params.hsl.hue[i] / 100.0) * 25.0;
                        sat_exp += we * params.hsl.saturation[i] / 100.0 * 0.7;
                        lum_exp += we * params.hsl.luminance[i] / 100.0 * 0.4;
                    }
                }
                h_deg += hue_shift;
                let c = c * crate::math::exp2(sat_exp);
                l *= crate::math::exp2(lum_exp);
                let h_rad = h_deg.to_radians();
                a = c * crate::math::cos(h_rad);
                b = c * crate::math::sin(h_rad);
            }

            // 2. Grading wheels (zone weights from the current Oklab L).
            if wheels_active {
                let w_s = 1.0 - smoothstep(0.25 + bal, 0.55 + bal, l);
                let w_h = smoothstep(0.45 + bal, 0.75 + bal, l);
                let w_m = (1.0 - w_s - w_h).max(0.0);
                for (w, off) in [w_s, w_m, w_h].into_iter().zip(wheels.iter()) {
                    a += w * off.da;
                    b += w * off.db;
                    l *= crate::math::exp2(w * off.l_exp);
                }
            }

            // 3. Vibrance (factor from pre-vibrance chroma).
            if params.vibrance != 0.0 {
                let c = (a * a + b * b).sqrt();
                let f = vibrance_factor(c, params.vibrance);
                a *= f;
                b *= f;
            }

            // 4. Saturation.
            if params.saturation != 0.0 {
                let f = saturation_factor(params.saturation);
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

/// Linear Rec.2020 → Oklab (through CIE XYZ D65).
pub(crate) fn rec2020_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    xyz_to_oklab(REC2020_TO_XYZ.mul_vec(rgb))
}

/// Oklab → linear Rec.2020 (through CIE XYZ D65). Inverse of
/// [`rec2020_to_oklab`]; output is unbounded.
pub(crate) fn oklab_to_rec2020(lab: [f32; 3]) -> [f32; 3] {
    XYZ_TO_REC2020.mul_vec(oklab_to_xyz(lab))
}

/// A grading wheel resolved to Oklab offsets (module docs):
/// `(Δa, Δb) = (sat/100)·0.08·(cos hue, sin hue)` and the lightness
/// exponent `lum/100·0.3`, each scaled by the zone weight at application.
pub(crate) struct WheelOffsets {
    /// Full-weight `a` offset.
    pub(crate) da: f32,
    /// Full-weight `b` offset.
    pub(crate) db: f32,
    /// Full-weight `log2` lightness gain.
    pub(crate) l_exp: f32,
}

impl WheelOffsets {
    /// True when the wheel changes anything.
    pub(crate) fn is_active(&self) -> bool {
        self.da != 0.0 || self.db != 0.0 || self.l_exp != 0.0
    }
}

/// Resolves a [`GradingWheel`] to its Oklab offsets.
pub(crate) fn wheel_offsets(wheel: &GradingWheel) -> WheelOffsets {
    let h = wheel.hue.to_radians();
    WheelOffsets {
        da: wheel.saturation / 100.0 * 0.08 * crate::math::cos(h),
        db: wheel.saturation / 100.0 * 0.08 * crate::math::sin(h),
        l_exp: wheel.luminance / 100.0 * 0.3,
    }
}

/// Vibrance chroma factor (module docs): low-chroma weighted, clamped ≥ 0.
pub(crate) fn vibrance_factor(chroma: f32, vibrance: f32) -> f32 {
    (1.0 + vibrance / 100.0 * 0.6 * (1.0 - (chroma / 0.35).clamp(0.0, 1.0))).max(0.0)
}

/// Saturation chroma factor (module docs): `1 + s/100·0.8`, clamped ≥ 0.
pub(crate) fn saturation_factor(saturation: f32) -> f32 {
    (1.0 + saturation / 100.0 * 0.8).max(0.0)
}

/// True when any HSL band slider is non-zero.
fn hsl_bands_active(hsl: &HslBands) -> bool {
    hsl.hue
        .iter()
        .chain(hsl.saturation.iter())
        .chain(hsl.luminance.iter())
        .any(|&v| v != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{oklab_to_oklch, oklch_to_oklab};

    /// Builds a Rec.2020 pixel from Oklch (hue in degrees).
    fn pixel_from_oklch(l: f32, c: f32, h_deg: f32) -> [f32; 3] {
        oklab_to_rec2020(oklch_to_oklab([l, c, h_deg.to_radians()]))
    }

    /// Oklch (hue in degrees, [0, 360)) of a Rec.2020 pixel.
    fn oklch_of(rgb: [f32; 3]) -> [f32; 3] {
        let [l, c, h] = oklab_to_oklch(rec2020_to_oklab(rgb));
        [l, c, h.to_degrees().rem_euclid(360.0)]
    }

    fn image_of(pixels: &[[f32; 3]]) -> ImageRgbF32 {
        let data: Vec<f32> = pixels.iter().flatten().copied().collect();
        ImageRgbF32::from_data(pixels.len() as u32, 1, data)
    }

    #[test]
    fn defaults_are_identity_within_roundtrip_tolerance() {
        let pixels = [
            [0.001, 0.002, 0.001],
            [0.1, 0.05, 0.2],
            [0.18, 0.18, 0.18],
            [0.5, 0.4, 0.3],
            [0.9, 0.95, 1.0],
            [1.5, 1.2, 0.8], // above white
        ];
        let mut img = image_of(&pixels);
        apply(&mut img, &ColorParams::default());
        for (i, px) in pixels.iter().enumerate() {
            let out = img.pixel(i as u32, 0);
            for c in 0..3 {
                let tol = 1e-4 * px[c].abs().max(0.05);
                assert!(
                    (out[c] - px[c]).abs() < tol,
                    "pixel {i} channel {c}: {} vs {}",
                    out[c],
                    px[c]
                );
            }
        }
    }

    #[test]
    fn disabled_stage_is_bit_exact_untouched() {
        let mut img = image_of(&[[0.3, 0.2, 0.1]]);
        let before = img.data().to_vec();
        let params = ColorParams {
            enabled: false,
            saturation: 100.0,
            ..ColorParams::default()
        };
        apply(&mut img, &params);
        assert_eq!(img.data(), before.as_slice());
    }

    #[test]
    fn non_finite_and_non_positive_l_pass_through() {
        let mut img = image_of(&[[f32::NAN, 0.5, 0.5], [0.0, 0.0, 0.0], [-0.2, -0.1, -0.05]]);
        let before = img.data().to_vec();
        let params = ColorParams {
            saturation: 100.0,
            vibrance: 50.0,
            ..ColorParams::default()
        };
        apply(&mut img, &params);
        // NaN pixel skipped; black (L = 0) and negative (L < 0) untouched.
        assert!(img.data()[0].is_nan());
        assert_eq!(&img.data()[1..], &before[1..]);
    }

    #[test]
    fn full_negative_saturation_scales_chroma_by_a_fifth() {
        let input = pixel_from_oklch(0.6, 0.15, 120.0);
        let mut img = image_of(&[input]);
        let params = ColorParams {
            saturation: -100.0,
            ..ColorParams::default()
        };
        apply(&mut img, &params);
        let [l_in, c_in, _] = oklch_of(input);
        let [l_out, c_out, _] = oklch_of(img.pixel(0, 0));
        assert!(c_out < c_in, "must move grey-ward");
        assert!(
            (c_out / c_in - 0.2).abs() < 0.01,
            "s = −100 must scale chroma by 1 − 0.8: got {}",
            c_out / c_in
        );
        assert!((l_out - l_in).abs() < 1e-3, "lightness must be preserved");
    }

    #[test]
    fn red_band_hue_shifts_red_but_not_blue() {
        let red = pixel_from_oklch(0.6, 0.15, 30.0); // at the red band centre
        let blue = pixel_from_oklch(0.6, 0.15, 260.0); // at the blue band centre
        let mut img = image_of(&[red, blue]);
        let mut hue = [0.0; HSL_BAND_COUNT];
        hue[0] = 100.0; // red band
        let params = ColorParams {
            hsl: HslBands {
                hue,
                ..HslBands::default()
            },
            ..ColorParams::default()
        };
        apply(&mut img, &params);

        let red_hue = oklch_of(img.pixel(0, 0))[2];
        // Full weight at the centre, e = 1 for C = 0.15: shift = 25°.
        assert!(
            (red_hue - 55.0).abs() < 0.5,
            "red patch must shift by ≈ 25°: {red_hue}"
        );
        let blue_hue = oklch_of(img.pixel(1, 0))[2];
        assert!(
            (blue_hue - 260.0).abs() < 0.1,
            "blue patch must not shift: {blue_hue}"
        );
    }

    #[test]
    fn near_neutral_pixels_resist_hsl_bands() {
        // C = 0.005 → engagement e ≈ 0: hue/sat sliders barely act.
        let muted = pixel_from_oklch(0.5, 0.005, 30.0);
        let mut img = image_of(&[muted]);
        let mut saturation = [0.0; HSL_BAND_COUNT];
        saturation[0] = 100.0;
        let params = ColorParams {
            hsl: HslBands {
                saturation,
                ..HslBands::default()
            },
            ..ColorParams::default()
        };
        apply(&mut img, &params);
        let [_, c_out, _] = oklch_of(img.pixel(0, 0));
        assert!(
            (c_out / 0.005 - 1.0).abs() < 0.02,
            "near-neutral chroma must be almost unchanged: {c_out}"
        );
    }

    #[test]
    fn shadow_wheel_tints_shadows_not_highlights() {
        let dark = pixel_from_oklch(0.2, 0.02, 100.0);
        let bright = pixel_from_oklch(0.9, 0.02, 100.0);
        let mut img = image_of(&[dark, bright]);
        let params = ColorParams {
            grading: crate::params::color::ColorGrading {
                shadows: GradingWheel {
                    hue: 0.0, // +a direction
                    saturation: 100.0,
                    luminance: 0.0,
                },
                ..Default::default()
            },
            ..ColorParams::default()
        };
        apply(&mut img, &params);

        let a_in_dark = rec2020_to_oklab(dark)[1];
        let a_out_dark = rec2020_to_oklab(img.pixel(0, 0))[1];
        assert!(
            (a_out_dark - a_in_dark - 0.08).abs() < 1e-3,
            "shadow zone gets the full +0.08 a offset: Δ = {}",
            a_out_dark - a_in_dark
        );

        let a_in_bright = rec2020_to_oklab(bright)[1];
        let a_out_bright = rec2020_to_oklab(img.pixel(1, 0))[1];
        assert!(
            (a_out_bright - a_in_bright).abs() < 1e-4,
            "highlight zone must be untouched by the shadow wheel"
        );
    }

    #[test]
    fn vibrance_boosts_muted_more_than_saturated() {
        let muted = pixel_from_oklch(0.6, 0.05, 140.0);
        let saturated = pixel_from_oklch(0.6, 0.4, 140.0);
        let mut img = image_of(&[muted, saturated]);
        let params = ColorParams {
            vibrance: 100.0,
            ..ColorParams::default()
        };
        apply(&mut img, &params);
        let muted_ratio = oklch_of(img.pixel(0, 0))[1] / 0.05;
        let saturated_ratio = oklch_of(img.pixel(1, 0))[1] / 0.4;
        assert!(
            muted_ratio > saturated_ratio + 0.2,
            "muted {muted_ratio} vs saturated {saturated_ratio}"
        );
        assert!(
            (saturated_ratio - 1.0).abs() < 0.02,
            "chroma ≥ 0.35 gets no vibrance boost: {saturated_ratio}"
        );
    }

    #[test]
    fn factor_helpers_reference_values() {
        assert_eq!(saturation_factor(0.0), 1.0);
        assert!((saturation_factor(50.0) - 1.4).abs() < 1e-6);
        assert!((saturation_factor(-100.0) - 0.2).abs() < 1e-6);
        // Vibrance is inert at high chroma, full-strength at zero chroma.
        assert_eq!(vibrance_factor(0.5, 100.0), 1.0);
        assert!((vibrance_factor(0.0, 100.0) - 1.6).abs() < 1e-6);
        assert!((vibrance_factor(0.0, -100.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn wheel_offsets_reference_values() {
        let w = wheel_offsets(&GradingWheel {
            hue: 90.0,
            saturation: 100.0,
            luminance: 100.0,
        });
        assert!(w.da.abs() < 1e-8, "hue 90° points along +b");
        assert!((w.db - 0.08).abs() < 1e-6);
        assert!((w.l_exp - 0.3).abs() < 1e-6);
        assert!(!wheel_offsets(&GradingWheel::default()).is_active());
    }
}
