//! Pipeline v1 stage 4: global tone — **frozen**: the formulas documented
//! here define the v1 output forever (HARD-VER).
//!
//! # Operating principle: luminance-ratio preservation
//!
//! All tone controls operate on a scalar per-pixel luminance and the RGB
//! triple is scaled by the resulting ratio, which preserves hue and
//! saturation (channel ratios) exactly:
//!
//! ```text
//! L    = luminance_rec2020(rgb)          (Rec.2020 Y coefficients)
//! L'   = tone_chain(L)                   (scalar chain below)
//! out  = rgb · (L' / L)
//! ```
//!
//! Pixels whose luminance is not strictly positive (black, out-of-gamut
//! negatives, NaN) receive **only** the exposure multiplier: the scalar
//! chain is undefined there, and scaling by `2^ev` is the unique tone
//! operation that extends continuously through zero.
//!
//! # The scalar chain (fixed order)
//!
//! Each operator is applied only when its parameter is non-neutral (slider
//! ≠ 0, curve non-identity); a neutral operator is **skipped**, which is the
//! pinned definition of identity for this stage (bit-exact pass-through at
//! default parameters).
//!
//! 1. **Exposure** (`ev` stops): `L′ = L · 2^ev`.
//! 2. **Contrast** (`c ∈ [−100, 100]`), a power curve pivoted at middle grey
//!    0.18 linear: `L′ = 0.18 · (L′ / 0.18)^g` with `g = 1 + c/150`.
//!    Monotonic for any `c` in range and exactly identity at `c = 0`.
//! 3. **Region controls** on the sRGB-encoded axis
//!    `l = srgb_encode(clamp(L′, 0, 1))` (IEC 61966-2-1); the block runs when
//!    any of the four sliders is non-zero:
//!    - shadows `s`:    `gain_s = 2^(s/100 · 0.9 · w_s)`,
//!      `w_s = 1 − smoothstep(0.05, 0.55, l)`
//!    - highlights `h`: `gain_h = 2^(h/100 · 0.8 · w_h)`,
//!      `w_h = smoothstep(0.45, 0.95, l)`, except `w_h = 1` when `L′ > 1`
//!    - `L′ ·= gain_s · gain_h`
//!    - whites `w`:     `L′ ·= 1 + w/100 · 0.25 · w_w`,
//!      `w_w = smoothstep(0.7, 1.0, min(l, 1))`, except `w_w = 1` when the
//!      current `L′ > 1` (evaluated after the shadow/highlight gains)
//!    - blacks `b`:     `l₂ = srgb_encode(clamp(L′, 0, 1))`;
//!      `L′ += b/100 · 0.06 · (1 − smoothstep(0.0, 0.35, l₂))`;
//!      then `L′ = max(L′, 0)`.
//!
//!    Values above 1.0 pass through the shadow and black weights unchanged
//!    (their weights are 0 at `l = 1`) while the highlight and white weights
//!    saturate to 1, so unbounded highlights keep responding to those two
//!    controls.
//! 4. **Point curve** (skipped when [`ToneCurve::is_identity`]), evaluated on
//!    the sRGB-encoded axis with monotone-cubic interpolation ([`Curve`]):
//!    - `L′ ≤ 1`: `L′ = srgb_decode(curve(srgb_encode(L′)))`
//!    - `L′ > 1`: `L′ = srgb_decode(curve(1)) · L′` — the pinned
//!      unbounded-highlight policy: above-white values are scaled linearly by
//!      the factor the curve applies to white, so the curve cannot introduce
//!      a discontinuity at 1.0 when `curve(1) = 1`.
//!
//! # Determinism
//!
//! Pixel maths is `f32` with the fixed expression order written here; `2^x`
//! is `f32::exp2`, the power curve is `f32::powf` (platform-libm caveat
//! documented in [`crate::color`]). Parallelism is `rayon` over disjoint
//! rows (`par_chunks_mut` with an exact row stride); every output pixel
//! depends only on its own input, so thread count cannot change results.

use rayon::prelude::*;

use crate::color::{luminance_rec2020, srgb_decode, srgb_encode};
use crate::image::ImageRgbF32;
use crate::params::tone::{CurvePoint, ToneCurve, ToneParams};

/// Applies the global tone stage in place (working space: linear Rec.2020,
/// unbounded). Does nothing when `params.enabled` is false.
pub fn apply(image: &mut ImageRgbF32, params: &ToneParams) {
    if !params.enabled {
        return;
    }
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

/// Hermite smoothstep: `t²(3 − 2t)` with `t = clamp((x − e0)/(e1 − e0), 0, 1)`.
///
/// Requires `e0 < e1` (all call sites use pinned constant edges).
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The precomputed tone chain (module docs) — shared by the global tone
/// stage and by local adjustments (stage 6), which build it from their
/// delta values.
pub(crate) struct ToneOps {
    /// `2^ev`, also the sole operation applied to non-positive luminances.
    pub(crate) exposure_mul: f32,
    /// Contrast slider value; 0 skips the operator.
    contrast: f32,
    /// Contrast exponent `g = 1 + contrast/150`.
    gamma: f32,
    /// Highlights slider.
    highlights: f32,
    /// Shadows slider.
    shadows: f32,
    /// Whites slider.
    whites: f32,
    /// Blacks slider.
    blacks: f32,
    /// The point curve, `None` when identity (operator skipped).
    curve: Option<Curve>,
    /// `srgb_decode(curve(1))`: the linear factor applied to `L′ > 1`.
    curve_white: f32,
}

impl ToneOps {
    /// Builds the chain from slider values (global params or local deltas).
    pub(crate) fn new(
        exposure: f32,
        contrast: f32,
        highlights: f32,
        shadows: f32,
        whites: f32,
        blacks: f32,
        curve: &ToneCurve,
    ) -> Self {
        let curve = (!curve.is_identity()).then(|| Curve::from_points(&curve.points));
        let curve_white = curve.as_ref().map_or(1.0, |c| srgb_decode(c.eval(1.0)));
        Self {
            exposure_mul: crate::math::exp2(exposure),
            contrast,
            gamma: 1.0 + contrast / 150.0,
            highlights,
            shadows,
            whites,
            blacks,
            curve,
            curve_white,
        }
    }

    /// Runs the scalar chain (module docs) on a strictly positive luminance.
    fn eval_luminance(&self, l: f32) -> f32 {
        // 1. Exposure.
        let mut lp = l * self.exposure_mul;

        // 2. Contrast (power curve pivoted at 0.18; skipped at 0).
        if self.contrast != 0.0 {
            lp = 0.18 * crate::math::powf(lp / 0.18, self.gamma);
        }

        // 3. Region controls (skipped when all four sliders are 0).
        if self.shadows != 0.0 || self.highlights != 0.0 || self.whites != 0.0 || self.blacks != 0.0
        {
            let l_enc = srgb_encode(lp); // srgb_encode clamps to [0, 1]
            let w_s = 1.0 - smoothstep(0.05, 0.55, l_enc);
            let gain_s = crate::math::exp2(self.shadows / 100.0 * 0.9 * w_s);
            let w_h = if lp > 1.0 {
                1.0
            } else {
                smoothstep(0.45, 0.95, l_enc)
            };
            let gain_h = crate::math::exp2(self.highlights / 100.0 * 0.8 * w_h);
            lp *= gain_s * gain_h;

            let w_w = if lp > 1.0 {
                1.0
            } else {
                smoothstep(0.7, 1.0, l_enc.min(1.0))
            };
            lp *= 1.0 + self.whites / 100.0 * 0.25 * w_w;

            let l2 = srgb_encode(lp);
            lp += self.blacks / 100.0 * 0.06 * (1.0 - smoothstep(0.0, 0.35, l2));
            lp = lp.max(0.0);
        }

        // 4. Point curve (skipped when identity).
        if let Some(curve) = &self.curve {
            if lp <= 1.0 {
                lp = srgb_decode(curve.eval(srgb_encode(lp)));
            } else {
                lp *= self.curve_white;
            }
        }

        lp
    }
}

/// Applies a prepared [`ToneOps`] chain to every pixel (module docs:
/// luminance-ratio preservation; non-positive luminance gets exposure only).
pub(crate) fn apply_tone_ops(image: &mut ImageRgbF32, ops: &ToneOps) {
    let stride = image.width() as usize * 3;
    if stride == 0 {
        return;
    }
    image.data_mut().par_chunks_mut(stride).for_each(|row| {
        for px in row.chunks_exact_mut(3) {
            let l = luminance_rec2020([px[0], px[1], px[2]]);
            if l > 0.0 {
                let ratio = ops.eval_luminance(l) / l;
                px[0] *= ratio;
                px[1] *= ratio;
                px[2] *= ratio;
            } else {
                px[0] *= ops.exposure_mul;
                px[1] *= ops.exposure_mul;
                px[2] *= ops.exposure_mul;
            }
        }
    });
}

/// A monotone cubic interpolant over sorted control points, per
/// Fritsch & Carlson, "Monotone Piecewise Cubic Interpolation",
/// SIAM J. Numer. Anal. 17(2), 1980 (the standard two-pass tangent
/// limiting; see also the "Monotone cubic interpolation" reference
/// algorithm).
///
/// Construction (pinned):
/// - non-finite points are dropped;
/// - points are stably sorted by `x`; among duplicate `x` the **first**
///   (pre-sort order) is kept;
/// - 0 points → the identity function; 1 point → the constant `y₀`;
/// - tangents: `mᵢ = (dᵢ₋₁ + dᵢ)/2` for interior points (0 where the secant
///   slopes `d` change sign), endpoint tangents equal the boundary secants,
///   then per interval the Fritsch–Carlson circle limiter: with
///   `α = mᵢ/dᵢ, β = mᵢ₊₁/dᵢ`, if `α² + β² > 9` both tangents are scaled by
///   `3/√(α² + β²)` (flat secants force both tangents to 0).
///
/// Evaluation clamps outside `[x₀, xₙ]` to the endpoint `y` values;
/// inside, cubic Hermite on the containing interval:
/// `y = h₀₀·yᵢ + h₁₀·h·mᵢ + h₀₁·yᵢ₊₁ + h₁₁·h·mᵢ₊₁` with `t = (x − xᵢ)/h`.
pub(crate) struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Tangent (dy/dx) at each control point.
    ms: Vec<f32>,
}

impl Curve {
    /// Builds the interpolant (see the type docs for the pinned rules).
    pub(crate) fn from_points(points: &[CurvePoint]) -> Curve {
        let mut pts: Vec<CurvePoint> = points
            .iter()
            .copied()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .collect();
        pts.sort_by(|a, b| a.x.partial_cmp(&b.x).expect("points are finite"));
        pts.dedup_by(|cur, kept| cur.x == kept.x);

        let n = pts.len();
        let xs: Vec<f32> = pts.iter().map(|p| p.x).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.y).collect();
        if n < 2 {
            return Curve {
                xs,
                ys,
                ms: vec![0.0; n],
            };
        }

        // Secant slopes per interval.
        let d: Vec<f32> = (0..n - 1)
            .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
            .collect();

        // Initial tangents.
        let mut ms = vec![0.0_f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                (d[i - 1] + d[i]) * 0.5
            };
        }

        // Fritsch–Carlson limiting, sequential over intervals (pinned order).
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
            } else {
                let a = ms[i] / d[i];
                let b = ms[i + 1] / d[i];
                let s = a * a + b * b;
                if s > 9.0 {
                    let t = 3.0 / s.sqrt();
                    ms[i] = t * a * d[i];
                    ms[i + 1] = t * b * d[i];
                }
            }
        }

        Curve { xs, ys, ms }
    }

    /// Evaluates the interpolant at `x` (type docs).
    pub(crate) fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if n == 0 {
            return x; // identity
        }
        if n == 1 {
            return self.ys[0]; // constant
        }
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        // Containing interval: the last i with xs[i] <= x; the guards above
        // pin i to [0, n − 2].
        let i = self.xs.partition_point(|&xi| xi <= x) - 1;
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[i] + h10 * h * self.ms[i] + h01 * self.ys[i + 1] + h11 * h * self.ms[i + 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe set spanning shadows, midtones, unbounded highlights, and
    /// non-positive luminance.
    fn probe_image() -> ImageRgbF32 {
        let pixels: Vec<[f32; 3]> = vec![
            [0.0, 0.0, 0.0],
            [0.01, 0.02, 0.005],
            [0.1, 0.05, 0.2],
            [0.18, 0.18, 0.18],
            [0.5, 0.4, 0.3],
            [0.9, 0.95, 1.0],
            [1.5, 2.0, 3.0], // > 1: unbounded working space
            [-0.05, 0.01, 0.02],
        ];
        let data: Vec<f32> = pixels.into_iter().flatten().collect();
        ImageRgbF32::from_data(4, 2, data)
    }

    #[test]
    fn defaults_are_bit_exact_identity() {
        let mut img = probe_image();
        let before = img.data().to_vec();
        apply(&mut img, &ToneParams::default());
        assert_eq!(img.data(), before.as_slice());
    }

    #[test]
    fn disabled_stage_is_untouched() {
        let mut img = probe_image();
        let before = img.data().to_vec();
        let params = ToneParams {
            enabled: false,
            exposure: 3.0,
            contrast: 80.0,
            ..ToneParams::default()
        };
        apply(&mut img, &params);
        assert_eq!(img.data(), before.as_slice());
    }

    #[test]
    fn exposure_plus_one_ev_doubles() {
        let mut img = probe_image();
        let before = img.clone();
        let params = ToneParams {
            exposure: 1.0,
            ..ToneParams::default()
        };
        apply(&mut img, &params);
        for (a, b) in img.data().iter().zip(before.data().iter()) {
            assert!(
                (a - 2.0 * b).abs() <= 1e-6 * b.abs().max(1.0),
                "{a} != 2·{b}"
            );
        }
    }

    #[test]
    fn contrast_pivots_at_middle_grey_and_stays_monotonic() {
        let params = ToneParams {
            contrast: 50.0,
            ..ToneParams::default()
        };

        // Below the 0.18 pivot: darker. Above: brighter. At: unchanged.
        let mut img = ImageRgbF32::from_data(
            3,
            1,
            vec![0.05, 0.05, 0.05, 0.18, 0.18, 0.18, 0.5, 0.5, 0.5],
        );
        apply(&mut img, &params);
        assert!(img.pixel(0, 0)[0] < 0.05);
        assert!((img.pixel(1, 0)[0] - 0.18).abs() < 1e-6);
        assert!(img.pixel(2, 0)[0] > 0.5);

        // Monotonic over an ascending grey ramp.
        let ramp: Vec<f32> = (0..64).flat_map(|i| [i as f32 / 32.0; 3]).collect();
        let mut img = ImageRgbF32::from_data(64, 1, ramp);
        apply(&mut img, &params);
        for x in 1..64 {
            assert!(
                img.pixel(x, 0)[0] >= img.pixel(x - 1, 0)[0],
                "contrast must be monotonic at x = {x}"
            );
        }
    }

    #[test]
    fn shadows_lift_dark_pixels_more_than_bright() {
        let params = ToneParams {
            shadows: 100.0,
            ..ToneParams::default()
        };
        let mut img = ImageRgbF32::from_data(2, 1, vec![0.02, 0.02, 0.02, 0.7, 0.7, 0.7]);
        apply(&mut img, &params);
        let dark_gain = img.pixel(0, 0)[0] / 0.02;
        let bright_gain = img.pixel(1, 0)[0] / 0.7;
        assert!(
            dark_gain > bright_gain,
            "dark gain {dark_gain} must exceed bright gain {bright_gain}"
        );
        assert!(
            dark_gain > 1.2,
            "shadows +100 must visibly lift: {dark_gain}"
        );
    }

    #[test]
    fn highlights_pull_down_bright_pixels_only() {
        let params = ToneParams {
            highlights: -100.0,
            ..ToneParams::default()
        };
        let mut img = ImageRgbF32::from_data(2, 1, vec![0.02, 0.02, 0.02, 0.9, 0.9, 0.9]);
        apply(&mut img, &params);
        assert!(
            (img.pixel(0, 0)[0] - 0.02).abs() < 1e-4,
            "shadows untouched"
        );
        assert!(img.pixel(1, 0)[0] < 0.9, "highlights recovered");
    }

    #[test]
    fn identity_curve_is_no_op_and_s_curve_adds_contrast() {
        let mut img = probe_image();
        let before = img.data().to_vec();
        let params = ToneParams::default(); // identity curve
        apply(&mut img, &params);
        assert_eq!(img.data(), before.as_slice());

        // Classic s-curve on the encoded axis.
        let s_curve = ToneCurve {
            points: vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 0.25, y: 0.2 },
                CurvePoint { x: 0.75, y: 0.8 },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        };
        let params = ToneParams {
            curve: s_curve,
            ..ToneParams::default()
        };
        let mut img = ImageRgbF32::from_data(2, 1, vec![0.03, 0.03, 0.03, 0.6, 0.6, 0.6]);
        apply(&mut img, &params);
        assert!(img.pixel(0, 0)[0] < 0.03, "s-curve darkens shadows");
        assert!(img.pixel(1, 0)[0] > 0.6, "s-curve brightens highlights");
    }

    #[test]
    fn curve_scales_above_white_by_endpoint_value() {
        // curve(1) = 0.5 on the encoded axis → L' > 1 scales by
        // srgb_decode(0.5).
        let curve = ToneCurve {
            points: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.5 }],
        };
        let params = ToneParams {
            curve,
            ..ToneParams::default()
        };
        let mut img = ImageRgbF32::from_data(1, 1, vec![2.0, 2.0, 2.0]);
        apply(&mut img, &params);
        let expected = srgb_decode(0.5) * 2.0;
        assert!((img.pixel(0, 0)[0] - expected).abs() < 1e-5);
    }

    #[test]
    fn ratio_scaling_preserves_channel_ratios() {
        let params = ToneParams {
            exposure: 0.7,
            contrast: 40.0,
            shadows: 30.0,
            highlights: -20.0,
            whites: 10.0,
            blacks: -10.0,
            ..ToneParams::default()
        };
        let rgb = [0.4_f32, 0.2, 0.1];
        let mut img = ImageRgbF32::from_data(1, 1, rgb.to_vec());
        apply(&mut img, &params);
        let out = img.pixel(0, 0);
        assert!(
            (out[0] / out[1] - rgb[0] / rgb[1]).abs() < 1e-4,
            "r/g ratio must be preserved"
        );
        assert!(
            (out[1] / out[2] - rgb[1] / rgb[2]).abs() < 1e-4,
            "g/b ratio must be preserved"
        );
    }

    #[test]
    fn non_positive_luminance_gets_exposure_only() {
        let params = ToneParams {
            exposure: 1.0,
            contrast: 80.0,
            shadows: 100.0,
            ..ToneParams::default()
        };
        // Zero pixel: stays zero.
        // Negative-luminance pixel: exactly doubled, nothing else.
        let mut img = ImageRgbF32::from_data(2, 1, vec![0.0, 0.0, 0.0, -0.1, 0.0, 0.0]);
        apply(&mut img, &params);
        assert_eq!(img.pixel(0, 0), [0.0, 0.0, 0.0]);
        assert_eq!(img.pixel(1, 0), [-0.2, 0.0, 0.0]);
    }

    #[test]
    fn region_controls_respond_above_white() {
        // Highlights and whites keep acting on L' > 1 (weights pinned to 1).
        let params = ToneParams {
            highlights: -100.0,
            ..ToneParams::default()
        };
        let mut img = ImageRgbF32::from_data(1, 1, vec![3.0, 3.0, 3.0]);
        apply(&mut img, &params);
        let expected = 3.0 * crate::math::exp2(-0.8_f32);
        assert!((img.pixel(0, 0)[0] - expected).abs() < 1e-4);
    }

    // --- Curve unit tests -------------------------------------------------

    #[test]
    fn curve_degenerate_cases() {
        let identity = Curve::from_points(&[]);
        assert_eq!(identity.eval(0.3), 0.3);
        assert_eq!(identity.eval(-2.0), -2.0);

        let constant = Curve::from_points(&[CurvePoint { x: 0.5, y: 0.25 }]);
        assert_eq!(constant.eval(0.0), 0.25);
        assert_eq!(constant.eval(1.0), 0.25);
    }

    #[test]
    fn curve_interpolates_through_control_points() {
        let pts = [
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.3, y: 0.5 },
            CurvePoint { x: 1.0, y: 0.8 },
        ];
        let c = Curve::from_points(&pts);
        for p in &pts {
            assert!((c.eval(p.x) - p.y).abs() < 1e-6, "must pass through {p:?}");
        }
    }

    #[test]
    fn curve_clamps_outside_domain() {
        let c = Curve::from_points(&[CurvePoint { x: 0.2, y: 0.1 }, CurvePoint { x: 0.8, y: 0.9 }]);
        assert_eq!(c.eval(0.0), 0.1);
        assert_eq!(c.eval(1.0), 0.9);
    }

    #[test]
    fn curve_is_monotone_for_monotone_points() {
        // A hard case for naive cubic splines (overshoot near flat spots).
        let c = Curve::from_points(&[
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.4, y: 0.05 },
            CurvePoint { x: 0.5, y: 0.9 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]);
        let mut prev = c.eval(0.0);
        for i in 1..=1000 {
            let y = c.eval(i as f32 / 1000.0);
            assert!(
                y >= prev - 1e-6,
                "monotone input must give monotone output at i = {i}"
            );
            prev = y;
        }
    }

    #[test]
    fn curve_sorts_points_defensively() {
        let sorted = Curve::from_points(&[
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.6 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]);
        let shuffled = Curve::from_points(&[
            CurvePoint { x: 1.0, y: 1.0 },
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.6 },
        ]);
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert_eq!(sorted.eval(x), shuffled.eval(x));
        }
    }

    #[test]
    fn curve_flat_segment_stays_flat() {
        let c = Curve::from_points(&[
            CurvePoint { x: 0.0, y: 0.5 },
            CurvePoint { x: 0.5, y: 0.5 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]);
        assert!((c.eval(0.25) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn smoothstep_reference_values() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.5), 0.5);
        assert_eq!(smoothstep(0.0, 1.0, 1.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert!((smoothstep(0.2, 0.6, 0.4) - 0.5).abs() < 1e-6);
    }
}
