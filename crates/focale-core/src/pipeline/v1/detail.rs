//! Pipeline v1 stage 7: detail — noise reduction and capture sharpening.
//!
//! **Frozen v1 algorithm** (PRD §2.2): every constant, kernel shape, and
//! iteration order in this module is pinned; changing any output requires a
//! new pipeline version.
//!
//! # Processing model
//!
//! The stage converts the linear Rec.2020 working image to Oklab planes
//! `L`, `a`, `b` via CIE XYZ ([`crate::color::xyz_to_oklab`]), processes
//! them, and converts back. The order is **noise reduction first, then
//! sharpening** — sharpening amplified noise would defeat the noise
//! reduction, so the sharpener always sees the denoised `L` plane.
//!
//! The Oklab round trip is unbounded: out-of-gamut and >1 working-space
//! values pass through unclamped. Pixels containing any non-finite channel
//! are excluded from processing and pass through unchanged; their plane
//! values are treated as `L = a = b = 0` when they appear inside a
//! neighbour's kernel.
//!
//! When the whole stage is disabled, or every sub-operation is at zero
//! strength, the image is left untouched (bit-exact identity). When any
//! sub-operation runs, untouched pixels still undergo the (near-lossless
//! but not bit-exact) Oklab round trip.
//!
//! # Determinism
//!
//! All pixel maths is `f32` in fixed expression order. `rayon` is used only
//! as disjoint-row parallelism reading immutable inputs; kernel sums are
//! accumulated sequentially in scan order (dy outer, dx inner). See the
//! transcendental-function caveat in [`crate::color`].

use rayon::prelude::*;

use crate::color::{REC2020_TO_XYZ, XYZ_TO_REC2020, oklab_to_xyz, xyz_to_oklab};
use crate::image::ImageRgbF32;
use crate::params::detail::{DetailParams, SharpenMethod};

/// Applies the detail stage (noise reduction, then sharpening) in place.
///
/// Skipped entirely (bit-exact identity) when `!params.enabled` or when all
/// of `noise_reduction.luminance`, `noise_reduction.chroma` and
/// `sharpen.amount` are ≤ 0.
///
/// `scale` is the preview-to-native resolution ratio; every
/// pixel-dimensioned parameter (blur σ, kernel radii) is multiplied by it
/// so previews stay perceptually faithful to the export
/// ([`crate::pipeline::RenderInput::scale`]).
///
/// # Frozen v1 formulas
///
/// **Luminance NR** (`nr.luminance` = n > 0): joint bilateral filter on the
/// `L` plane with spatial `σ_s = (1 + n/100 · 1.5) · scale` px, range
/// `σ_r = n/100 · 0.08` (Oklab L units), square kernel of radius
/// `ceil(2·σ_s)` capped at 16 px, edge-clamped. Per-neighbour weight
/// `w = exp(−d²/(2σ_s²) − ΔL²/(2σ_r²))` where `d` is the spatial offset and
/// `ΔL` the L difference to the centre pixel; the guide is the input `L`
/// plane itself. Detail blend-back with `d = nr.luminance_detail`:
/// `L_out = filtered + (L_in − filtered) · (d/100 · 0.6)`.
///
/// **Chroma NR** (`nr.chroma` = n > 0): plain separable Gaussian on the
/// `a` and `b` planes with `σ = n/100 · 3.0 · scale` px, kernel radius
/// `ceil(3σ)` capped at 24 px. Blend-back identical in form to luminance:
/// `out = blurred + (in − blurred) · (nr.chroma_detail/100 · 0.6)`.
///
/// **Sharpening** (`sharpen.amount` = A > 0) operates on the (possibly
/// denoised) `L` plane with `σ = sharpen.radius · scale`. Edge mask when
/// `sharpen.masking` = M > 0: gradient magnitude `g = |∇L|` from central
/// differences (edge-clamped, so borders degrade to one-sided halved
/// differences), `m = smoothstep(t0, t1, g)` with `t0 = M/100 · 0.02` and
/// `t1 = 4·t0` (a fixed 1:4 threshold ramp — gradients below `t0` are fully
/// spared, above `4·t0` fully sharpened). `m = 1` everywhere when M = 0.
///
/// - *Unsharp*: `blur = gaussian(L, σ)` (kernel radius `ceil(3σ)`, capped
///   at 64 px defensively), `high = L − blur`,
///   `L += A/100 · 1.2 · high · m`.
/// - *Deconvolution*: Richardson–Lucy with a Gaussian PSF `K` of the same
///   σ, **exactly 10 iterations**, on the positive-clamped plane
///   `d = max(L, 10⁻⁶)`: starting from `u₀ = d`,
///   `u_{k+1} = u_k · (K ⊗ (d / max(K ⊗ u_k, 10⁻⁶)))`
///   (Richardson 1972 / Lucy 1974; the Gaussian PSF is symmetric so
///   correlation equals convolution). Result blended:
///   `L += A/100 · (u₁₀ − L) · m`.
pub fn apply(image: &mut ImageRgbF32, params: &DetailParams, scale: f32) {
    if !params.enabled {
        return;
    }
    let nr = &params.noise_reduction;
    let sharpen = &params.sharpen;
    let do_luma = nr.luminance > 0.0;
    let do_chroma = nr.chroma > 0.0;
    let do_sharpen = sharpen.amount > 0.0;
    if !do_luma && !do_chroma && !do_sharpen {
        return;
    }

    let w = image.width() as usize;
    let h = image.height() as usize;
    if w == 0 || h == 0 {
        return;
    }
    let n = w * h;

    // Decompose into Oklab planes. Non-finite pixels are flagged and left
    // as L = a = b = 0 in the planes (documented above).
    let mut plane_l = vec![0.0_f32; n];
    let mut plane_a = vec![0.0_f32; n];
    let mut plane_b = vec![0.0_f32; n];
    let mut finite = vec![true; n];
    {
        let data = image.data();
        plane_l
            .par_chunks_mut(w)
            .zip(
                plane_a
                    .par_chunks_mut(w)
                    .zip(plane_b.par_chunks_mut(w).zip(finite.par_chunks_mut(w))),
            )
            .enumerate()
            .for_each(|(y, (lr, (ar, (br, fr))))| {
                let row = &data[y * w * 3..(y + 1) * w * 3];
                for x in 0..w {
                    let rgb = [row[x * 3], row[x * 3 + 1], row[x * 3 + 2]];
                    if rgb[0].is_finite() && rgb[1].is_finite() && rgb[2].is_finite() {
                        let lab = xyz_to_oklab(REC2020_TO_XYZ.mul_vec(rgb));
                        lr[x] = lab[0];
                        ar[x] = lab[1];
                        br[x] = lab[2];
                    } else {
                        fr[x] = false;
                    }
                }
            });
    }

    // 1. Noise reduction (luma, then chroma).
    if do_luma {
        let sigma_s = (1.0 + nr.luminance / 100.0 * 1.5) * scale;
        let sigma_r = nr.luminance / 100.0 * 0.08;
        let radius = ((2.0 * sigma_s).ceil() as usize).clamp(1, 16);
        let filtered = bilateral(&plane_l, w, h, radius, sigma_s, sigma_r);
        blend_detail(&mut plane_l, &filtered, nr.luminance_detail / 100.0 * 0.6);
    }
    if do_chroma {
        let sigma = nr.chroma / 100.0 * 3.0 * scale;
        let detail = nr.chroma_detail / 100.0 * 0.6;
        let blurred = gaussian_blur_plane(&plane_a, w, h, sigma, 24);
        blend_detail(&mut plane_a, &blurred, detail);
        let blurred = gaussian_blur_plane(&plane_b, w, h, sigma, 24);
        blend_detail(&mut plane_b, &blurred, detail);
    }

    // 2. Sharpening on the (possibly denoised) L plane.
    if do_sharpen {
        let sigma = sharpen.radius * scale;
        let mask = if sharpen.masking > 0.0 {
            Some(edge_mask(&plane_l, w, h, sharpen.masking))
        } else {
            None
        };
        match sharpen.method {
            SharpenMethod::Unsharp => {
                let blur = gaussian_blur_plane(&plane_l, w, h, sigma, 64);
                let k = sharpen.amount / 100.0 * 1.2;
                for i in 0..n {
                    let m = mask.as_ref().map_or(1.0, |mk| mk[i]);
                    plane_l[i] += k * (plane_l[i] - blur[i]) * m;
                }
            }
            SharpenMethod::Deconvolution => {
                let deconvolved = richardson_lucy(&plane_l, w, h, sigma);
                let k = sharpen.amount / 100.0;
                for i in 0..n {
                    let m = mask.as_ref().map_or(1.0, |mk| mk[i]);
                    plane_l[i] += k * (deconvolved[i] - plane_l[i]) * m;
                }
            }
        }
    }

    // Recompose. Out-of-gamut stays unbounded; non-finite pixels pass
    // through untouched.
    let data = image.data_mut();
    data.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let i = y * w + x;
            if !finite[i] {
                continue;
            }
            let rgb = XYZ_TO_REC2020.mul_vec(oklab_to_xyz([plane_l[i], plane_a[i], plane_b[i]]));
            row[x * 3] = rgb[0];
            row[x * 3 + 1] = rgb[1];
            row[x * 3 + 2] = rgb[2];
        }
    });
}

/// Hermite smoothstep: 0 for `x ≤ e0`, 1 for `x ≥ e1`, else `t²(3 − 2t)`
/// with `t = (x − e0)/(e1 − e0)`. Callers guarantee `e1 > e0`.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Detail blend-back: `base = filtered + (base − filtered) · detail`,
/// element-wise in scan order.
fn blend_detail(base: &mut [f32], filtered: &[f32], detail: f32) {
    for (o, f) in base.iter_mut().zip(filtered.iter()) {
        *o = f + (*o - f) * detail;
    }
}

/// Joint bilateral filter on a single plane (guide = the plane itself).
///
/// Square kernel of the given radius, edge-clamped. Weights
/// `w = exp(−(dx² + dy²)/(2σ_s²) − ΔL²/(2σ_r²))`, accumulated sequentially
/// with dy as the outer loop and dx inner. The centre weight is 1, so the
/// normalizer is never zero.
fn bilateral(
    src: &[f32],
    w: usize,
    h: usize,
    radius: usize,
    sigma_s: f32,
    sigma_r: f32,
) -> Vec<f32> {
    let inv_2ss = 1.0 / (2.0 * sigma_s * sigma_s);
    let inv_2sr = 1.0 / (2.0 * sigma_r * sigma_r);
    let r = radius as isize;
    let mut out = vec![0.0_f32; src.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, orow)| {
        for x in 0..w {
            let centre = src[y * w + x];
            let mut num = 0.0_f32;
            let mut den = 0.0_f32;
            for dy in -r..=r {
                let sy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                for dx in -r..=r {
                    let sx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                    let v = src[sy * w + sx];
                    let d2 = (dx * dx + dy * dy) as f32;
                    let dl = v - centre;
                    let wgt = crate::math::exp(-(d2 * inv_2ss) - dl * dl * inv_2sr);
                    num += wgt * v;
                    den += wgt;
                }
            }
            orow[x] = num / den;
        }
    });
    out
}

/// Separable Gaussian blur on a plane.
///
/// Kernel radius `ceil(3σ)` capped at `max_radius`; weights
/// `exp(−i²/(2σ²))` normalized by their sequential sum. Horizontal pass
/// then vertical pass, edge-clamped, each parallel over disjoint output
/// rows reading an immutable input. Returns a copy when σ ≤ 0.
fn gaussian_blur_plane(src: &[f32], w: usize, h: usize, sigma: f32, max_radius: usize) -> Vec<f32> {
    if sigma.is_nan() || sigma <= 0.0 {
        return src.to_vec();
    }
    let radius = ((3.0 * sigma).ceil() as usize).min(max_radius);
    if radius == 0 {
        return src.to_vec();
    }
    let inv_2s2 = 1.0 / (2.0 * sigma * sigma);
    let mut weights = Vec::with_capacity(2 * radius + 1);
    for i in -(radius as isize)..=(radius as isize) {
        let fi = i as f32;
        weights.push(crate::math::exp(-(fi * fi) * inv_2s2));
    }
    let mut sum = 0.0_f32;
    for wgt in &weights {
        sum += *wgt; // sequential summation (determinism)
    }
    for wgt in &mut weights {
        *wgt /= sum;
    }

    let r = radius as isize;
    let mut tmp = vec![0.0_f32; src.len()];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, orow)| {
        let irow = &src[y * w..(y + 1) * w];
        for (x, out_px) in orow.iter_mut().enumerate() {
            let mut acc = 0.0_f32;
            for (k, wgt) in weights.iter().enumerate() {
                let sx = (x as isize + k as isize - r).clamp(0, w as isize - 1) as usize;
                acc += wgt * irow[sx];
            }
            *out_px = acc;
        }
    });
    let mut out = vec![0.0_f32; src.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, orow)| {
        for x in 0..w {
            let mut acc = 0.0_f32;
            for (k, wgt) in weights.iter().enumerate() {
                let sy = (y as isize + k as isize - r).clamp(0, h as isize - 1) as usize;
                acc += wgt * tmp[sy * w + x];
            }
            orow[x] = acc;
        }
    });
    out
}

/// Edge mask for sharpening: `m = smoothstep(t0, 4·t0, |∇L|)` with
/// `t0 = masking/100 · 0.02`, gradients from central differences
/// (edge-clamped indices).
fn edge_mask(l: &[f32], w: usize, h: usize, masking: f32) -> Vec<f32> {
    let t0 = masking / 100.0 * 0.02;
    let t1 = t0 * 4.0;
    let mut out = vec![0.0_f32; l.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, orow)| {
        let ym = y.saturating_sub(1);
        let yp = (y + 1).min(h - 1);
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let gx = (l[y * w + xp] - l[y * w + xm]) * 0.5;
            let gy = (l[yp * w + x] - l[ym * w + x]) * 0.5;
            let g = (gx * gx + gy * gy).sqrt();
            orow[x] = smoothstep(t0, t1, g);
        }
    });
    out
}

/// Richardson–Lucy deconvolution of a plane with a Gaussian PSF, exactly
/// 10 iterations (see [`apply`] for the pinned formula).
fn richardson_lucy(l: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let d: Vec<f32> = l.iter().map(|v| v.max(1e-6)).collect();
    let mut u = d.clone();
    for _ in 0..10 {
        let est = gaussian_blur_plane(&u, w, h, sigma, 64);
        let ratio: Vec<f32> = d
            .iter()
            .zip(est.iter())
            .map(|(dv, ev)| dv / ev.max(1e-6))
            .collect();
        let corr = gaussian_blur_plane(&ratio, w, h, sigma, 64);
        for (uv, cv) in u.iter_mut().zip(corr.iter()) {
            *uv *= cv;
        }
    }
    u
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::detail::{NoiseReductionParams, SharpenParams};

    fn flat(w: u32, h: u32, v: f32) -> ImageRgbF32 {
        ImageRgbF32::from_data(w, h, vec![v; w as usize * h as usize * 3])
    }

    /// Deterministic pseudo-noise in [−1, 1) (LCG; test-only).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32) / (2147483648.0 / 2.0) - 1.0
        }
    }

    fn nr_only(
        luminance: f32,
        luminance_detail: f32,
        chroma: f32,
        chroma_detail: f32,
    ) -> DetailParams {
        DetailParams {
            enabled: true,
            sharpen: SharpenParams {
                amount: 0.0,
                ..SharpenParams::default()
            },
            noise_reduction: NoiseReductionParams {
                luminance,
                luminance_detail,
                chroma,
                chroma_detail,
            },
        }
    }

    fn sharpen_only(method: SharpenMethod, amount: f32, radius: f32, masking: f32) -> DetailParams {
        DetailParams {
            enabled: true,
            sharpen: SharpenParams {
                method,
                amount,
                radius,
                masking,
            },
            noise_reduction: NoiseReductionParams::default(),
        }
    }

    fn variance(vals: impl Iterator<Item = f32>) -> f64 {
        let v: Vec<f64> = vals.map(f64::from).collect();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / v.len() as f64
    }

    #[test]
    fn disabled_is_bit_exact_identity() {
        let mut img = flat(8, 8, 0.5);
        img.set_pixel(3, 3, [0.9, 0.1, 0.4]);
        let orig = img.clone();
        let params = DetailParams {
            enabled: false,
            sharpen: SharpenParams {
                amount: 100.0,
                ..SharpenParams::default()
            },
            noise_reduction: NoiseReductionParams {
                luminance: 80.0,
                ..NoiseReductionParams::default()
            },
        };
        apply(&mut img, &params, 1.0);
        assert_eq!(img.data(), orig.data());
    }

    #[test]
    fn zero_amounts_are_bit_exact_identity() {
        let mut img = flat(8, 8, 0.5);
        img.set_pixel(1, 2, [1.7, 0.0, -0.2]); // unbounded working values
        let orig = img.clone();
        let params = nr_only(0.0, 50.0, 0.0, 50.0); // sharpen.amount = 0 too
        apply(&mut img, &params, 1.0);
        assert_eq!(img.data(), orig.data());
    }

    #[test]
    fn luma_nr_reduces_noise_and_flat_stays_flat() {
        let (w, h) = (48u32, 24u32);
        let mut img = flat(w, h, 0.5);
        let mut rng = Lcg(42);
        for y in 0..h {
            for x in 0..w {
                let noise = if x < 24 { rng.next() * 0.05 } else { 0.0 };
                let v = 0.5 + noise;
                img.set_pixel(x, y, [v, v, v]);
            }
        }
        let before: Vec<f32> = (4..20)
            .flat_map(|y| (4..20).map(move |x| (x, y)))
            .map(|(x, y)| img.pixel(x, y)[0])
            .collect();
        let mut out = img.clone();
        apply(&mut out, &nr_only(80.0, 0.0, 0.0, 0.0), 1.0);
        let after: Vec<f32> = (4..20)
            .flat_map(|y| (4..20).map(move |x| (x, y)))
            .map(|(x, y)| out.pixel(x, y)[0])
            .collect();
        let var_before = variance(before.into_iter());
        let var_after = variance(after.into_iter());
        assert!(
            var_after < var_before * 0.5,
            "NR should halve variance: before {var_before}, after {var_after}"
        );
        // The flat right side (away from the noisy region and any kernel
        // reach) must stay uniform and close to its original level.
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for y in 2..h - 2 {
            for x in 34..w - 2 {
                let v = out.pixel(x, y)[0];
                min = min.min(v);
                max = max.max(v);
            }
        }
        assert!(max - min < 1e-4, "flat region not uniform: {min}..{max}");
        assert!((f64::from(min) - 0.5).abs() < 5e-3);
    }

    #[test]
    fn chroma_nr_reduces_chroma_noise() {
        let (w, h) = (32u32, 32u32);
        let mut img = flat(w, h, 0.5);
        let mut rng = Lcg(7);
        for y in 0..h {
            for x in 0..w {
                let n = rng.next() * 0.05;
                img.set_pixel(x, y, [0.5 + n, 0.5, 0.5 - n]);
            }
        }
        let rb_var = |im: &ImageRgbF32| {
            variance(
                (4..28)
                    .flat_map(|y| (4..28).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let p = im.pixel(x, y);
                        p[0] - p[2]
                    }),
            )
        };
        let before = rb_var(&img);
        let mut out = img.clone();
        apply(&mut out, &nr_only(0.0, 0.0, 80.0, 0.0), 1.0);
        let after = rb_var(&out);
        assert!(
            after < before * 0.5,
            "chroma NR should reduce R−B variance: before {before}, after {after}"
        );
    }

    fn max_adjacent_diff(img: &ImageRgbF32, y: u32) -> f32 {
        let mut m = 0.0_f32;
        for x in 0..img.width() - 1 {
            m = m.max((img.pixel(x + 1, y)[0] - img.pixel(x, y)[0]).abs());
        }
        m
    }

    #[test]
    fn unsharp_increases_edge_contrast() {
        let (w, h) = (32u32, 16u32);
        let mut img = ImageRgbF32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if x < 16 { 0.2 } else { 0.6 };
                img.set_pixel(x, y, [v, v, v]);
            }
        }
        let before = max_adjacent_diff(&img, 8);
        let mut out = img.clone();
        apply(
            &mut out,
            &sharpen_only(SharpenMethod::Unsharp, 100.0, 1.0, 0.0),
            1.0,
        );
        let after = max_adjacent_diff(&out, 8);
        assert!(
            after > before,
            "unsharp must steepen the edge: {before} -> {after}"
        );
    }

    #[test]
    fn deconvolution_increases_edge_contrast() {
        let (w, h) = (32u32, 16u32);
        let mut img = ImageRgbF32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = match x {
                    0..=14 => 0.2,
                    15 => 0.3,
                    16 => 0.5,
                    _ => 0.6,
                };
                img.set_pixel(x, y, [v, v, v]);
            }
        }
        let before = max_adjacent_diff(&img, 8);
        let mut out = img.clone();
        apply(
            &mut out,
            &sharpen_only(SharpenMethod::Deconvolution, 100.0, 1.5, 0.0),
            1.0,
        );
        let after = max_adjacent_diff(&out, 8);
        assert!(
            after > before,
            "deconvolution must steepen the edge: {before} -> {after}"
        );
    }

    #[test]
    fn masking_spares_low_gradient_areas() {
        // Low-amplitude ripple on the left (below the mask threshold), a
        // strong step edge on the right.
        let (w, h) = (64u32, 16u32);
        let mut img = ImageRgbF32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if x < 32 {
                    0.3 + 0.01 * crate::math::sin((x as f32) * std::f32::consts::TAU / 8.0)
                } else if x < 48 {
                    0.3
                } else {
                    0.7
                };
                img.set_pixel(x, y, [v, v, v]);
            }
        }
        let run = |masking: f32| {
            let mut out = img.clone();
            apply(
                &mut out,
                &sharpen_only(SharpenMethod::Unsharp, 150.0, 1.0, masking),
                1.0,
            );
            out
        };
        let unmasked = run(0.0);
        let masked = run(100.0);
        let ripple_delta = |out: &ImageRgbF32| {
            let mut sum = 0.0_f64;
            for y in 2..h - 2 {
                for x in 4..28 {
                    sum += f64::from((out.pixel(x, y)[0] - img.pixel(x, y)[0]).abs());
                }
            }
            sum
        };
        let d_masked = ripple_delta(&masked);
        let d_unmasked = ripple_delta(&unmasked);
        assert!(
            d_masked < d_unmasked * 0.5,
            "masking must spare the ripple: masked {d_masked}, unmasked {d_unmasked}"
        );
        // The strong edge must still be sharpened under full masking.
        let mut edge_delta = 0.0_f32;
        for x in 46..51 {
            edge_delta = edge_delta.max((masked.pixel(x, 8)[0] - img.pixel(x, 8)[0]).abs());
        }
        assert!(edge_delta > 5e-3, "edge must still sharpen: {edge_delta}");
    }

    #[test]
    fn scale_shrinks_the_halo() {
        let (w, h) = (64u32, 8u32);
        let mut img = ImageRgbF32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if x < 32 { 0.2 } else { 0.6 };
                img.set_pixel(x, y, [v, v, v]);
            }
        }
        let changed = |scale: f32| {
            let mut out = img.clone();
            apply(
                &mut out,
                &sharpen_only(SharpenMethod::Unsharp, 100.0, 3.0, 0.0),
                scale,
            );
            (0..w)
                .filter(|&x| (out.pixel(x, 4)[0] - img.pixel(x, 4)[0]).abs() > 1e-3)
                .count()
        };
        let full = changed(1.0);
        let half = changed(0.5);
        assert!(
            half < full,
            "halo at scale 0.5 ({half} px) must be narrower than at 1.0 ({full} px)"
        );
    }

    #[test]
    fn non_finite_pixels_pass_through() {
        let mut img = flat(16, 16, 0.5);
        img.set_pixel(8, 8, [f32::NAN, 0.5, f32::INFINITY]);
        let mut out = img.clone();
        apply(&mut out, &nr_only(50.0, 50.0, 50.0, 50.0), 1.0);
        let p = out.pixel(8, 8);
        assert!(p[0].is_nan());
        assert_eq!(p[1], 0.5);
        assert_eq!(p[2], f32::INFINITY);
    }

    #[test]
    fn double_run_is_bit_identical() {
        let (w, h) = (40u32, 24u32);
        let mut img = ImageRgbF32::new(w, h);
        let mut rng = Lcg(99);
        for y in 0..h {
            for x in 0..w {
                img.set_pixel(
                    x,
                    y,
                    [
                        0.4 + rng.next() * 0.2,
                        0.4 + rng.next() * 0.2,
                        0.4 + rng.next() * 0.2,
                    ],
                );
            }
        }
        let params = DetailParams {
            enabled: true,
            sharpen: SharpenParams {
                method: SharpenMethod::Unsharp,
                amount: 80.0,
                radius: 1.5,
                masking: 30.0,
            },
            noise_reduction: NoiseReductionParams {
                luminance: 50.0,
                luminance_detail: 50.0,
                chroma: 40.0,
                chroma_detail: 30.0,
            },
        };
        let mut a = img.clone();
        let mut b = img.clone();
        apply(&mut a, &params, 1.0);
        apply(&mut b, &params, 1.0);
        assert_eq!(a.data(), b.data());
    }
}
