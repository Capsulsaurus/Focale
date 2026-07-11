//! Pipeline v1 stage 10: finishing — post-crop vignette, then grain.
//!
//! **Frozen v1 algorithm** (PRD §2.2). The vignette is applied first and
//! the grain second, so grain texture is not modulated by the vignette
//! falloff (it rides on the vignetted luminance instead).
//!
//! # Determinism
//!
//! Both effects are pure per-pixel functions of the output coordinate (and
//! the grain seed), evaluated in `f32` with fixed expression order;
//! parallelism is disjoint-row only. Grain derives entirely from
//! SplitMix64 hashes of the integer cell coordinates, so identical
//! sidecars render identical grain on any machine and any thread count.

use rayon::prelude::*;

use crate::color::{luminance_rec2020, srgb_decode, srgb_encode};
use crate::image::ImageRgbF32;
use crate::params::finishing::{FinishingParams, GrainParams, VignetteParams};

/// Applies the finishing stage (vignette, then grain) in place.
///
/// Skipped entirely (bit-exact identity) when `!params.enabled`; the
/// vignette runs only when `vignette.amount != 0` and grain only when
/// `grain.amount > 0`.
///
/// `scale` is the preview-to-native resolution ratio; the grain cell size
/// (the only pixel-dimensioned parameter here) is multiplied by it
/// ([`crate::pipeline::RenderInput::scale`]). The vignette is resolution-
/// independent by construction.
///
/// # Frozen v1 vignette formula
///
/// Post-crop, centred. For each pixel, normalized coordinates
/// `u = 2(x + 0.5)/W − 1`, `v = 2(y + 0.5)/H − 1` (so `u, v ∈ (−1, 1)`).
/// Distances: `d_circ = sqrt(u² + v²) · (1/√2)` (corner = 1) and
/// `d_rect = max(|u|, |v|)`. Roundness `r ∈ [−100, 100]` blends them with
/// `t = (r + 100)/200`: `d = d_rect + (d_circ − d_rect)·t`. Midpoint `m`
/// and feather `f` (both 0–100) place the falloff ramp:
/// `start = m/100 · 0.9`, `end = start + 0.1 + f/100 · 0.9`,
/// `falloff = smoothstep(start, end, d)`. Every channel is multiplied by
/// `factor = 2^(amount/100 · 1.5 · falloff)` — negative amounts darken
/// (negative exponent), positive brighten.
///
/// # Frozen v1 grain formula
///
/// Luminance-ratio noise applied in the sRGB-encoded domain, a pure
/// function of `(x, y, seed)`:
///
/// - Cell size `c = max(1, size/100 · 4 · scale)` px. The noise field is
///   sampled at `g = (pixel + 0.5)/c − 0.5` and bilinearly interpolated
///   between the four surrounding integer cell corners (at `c = 1` this
///   degenerates to one independent value per pixel).
/// - Cell value: with cell coords `(cx, cy)` as `i64` (two's-complement
///   cast to `u64`), the key is
///   `seed XOR (cx as u64 & 0xFFFF_FFFF) XOR ((cy as u64) << 32)`, hashed
///   by one SplitMix64 finalization step
///   (`z += 0x9E3779B97F4A7C15; z = (z ^ z>>30) · 0xBF58476D1CE4E5B9;
///   z = (z ^ z>>27) · 0x94D049BB133111EB; z ^= z>>31`), and the **top 24
///   bits** map linearly to `[−1, 1)`: `n = bits · 2⁻²³ − 1`.
/// - Roughness `ρ` adds a second octave at half cell size, hashed with
///   `seed XOR 0x9E3779B97F4A7C15`, weight `w₂ = ρ/100 · 0.5`,
///   renormalized: `n = (n₁ + w₂·n₂)/(1 + w₂)`.
/// - Application, midtone-weighted: with pixel luminance
///   `L = luminance_rec2020(rgb)`, grain applies **only when
///   `0 < L ≤ 1`** — pixels at or below black are untouched and unbounded
///   highlights (`L > 1`) are deliberately skipped so grain never eats
///   into HDR headroom (pinned v1 behaviour). Then
///   `l = srgb_encode(L)`,
///   `l' = clamp(l + n · amount/100 · 0.05 · (0.25 + 0.75·4l(1−l)), 0, 1)`,
///   `L' = srgb_decode(l')`, and the pixel is scaled by `L'/L` (the
///   midtone weight peaks at l = 0.5 and floors at 0.25 near black/white).
pub fn apply(image: &mut ImageRgbF32, params: &FinishingParams, scale: f32) {
    if !params.enabled {
        return;
    }
    if params.vignette.amount != 0.0 {
        vignette(image, &params.vignette);
    }
    if params.grain.amount > 0.0 {
        grain(image, &params.grain, scale);
    }
}

/// Hermite smoothstep: 0 for `x ≤ e0`, 1 for `x ≥ e1`, else `t²(3 − 2t)`
/// with `t = (x − e0)/(e1 − e0)`. Callers guarantee `e1 > e0`.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Post-crop vignette (formula on [`apply`]); parallel over disjoint rows.
fn vignette(image: &mut ImageRgbF32, p: &VignetteParams) {
    let w = image.width() as usize;
    let wf = image.width() as f32;
    let hf = image.height() as f32;
    let t = ((p.roundness + 100.0) / 200.0).clamp(0.0, 1.0);
    let start = p.midpoint / 100.0 * 0.9;
    let end = start + 0.1 + p.feather / 100.0 * 0.9;
    let k = p.amount / 100.0 * 1.5;
    image
        .data_mut()
        .par_chunks_mut(w * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let v = 2.0 * (y as f32 + 0.5) / hf - 1.0;
            for x in 0..w {
                let u = 2.0 * (x as f32 + 0.5) / wf - 1.0;
                let d_circ = (u * u + v * v).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
                let d_rect = u.abs().max(v.abs());
                let d = d_rect + (d_circ - d_rect) * t;
                let falloff = smoothstep(start, end, d);
                let factor = crate::math::exp2(k * falloff);
                row[x * 3] *= factor;
                row[x * 3 + 1] *= factor;
                row[x * 3 + 2] *= factor;
            }
        });
}

/// One SplitMix64 finalization step (Steele, Lea & Flood 2014), used as a
/// stateless hash of the cell key.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Cell corner noise in `[−1, 1)` (bit mixing pinned on [`apply`]).
fn cell_noise(cx: i64, cy: i64, seed: u64) -> f32 {
    let key = seed ^ (cx as u64 & 0xFFFF_FFFF) ^ ((cy as u64) << 32);
    let bits = (splitmix64(key) >> 40) as u32; // top 24 bits
    bits as f32 * (2.0 / 16_777_216.0) - 1.0
}

/// Bilinearly interpolated grain field at pixel `(x, y)` for the given
/// cell size (sampling convention pinned on [`apply`]).
fn sample_noise(x: usize, y: usize, cell: f32, seed: u64) -> f32 {
    let gx = (x as f32 + 0.5) / cell - 0.5;
    let gy = (y as f32 + 0.5) / cell - 0.5;
    let x0 = gx.floor();
    let y0 = gy.floor();
    let fx = gx - x0;
    let fy = gy - y0;
    let cx = x0 as i64;
    let cy = y0 as i64;
    let n00 = cell_noise(cx, cy, seed);
    let n10 = cell_noise(cx + 1, cy, seed);
    let n01 = cell_noise(cx, cy + 1, seed);
    let n11 = cell_noise(cx + 1, cy + 1, seed);
    let top = n00 * (1.0 - fx) + n10 * fx;
    let bottom = n01 * (1.0 - fx) + n11 * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Seeded procedural grain (formula on [`apply`]); parallel over disjoint
/// rows.
fn grain(image: &mut ImageRgbF32, p: &GrainParams, scale: f32) {
    let w = image.width() as usize;
    let cell = (p.size / 100.0 * 4.0 * scale).max(1.0);
    let half_cell = cell * 0.5;
    let w2 = p.roughness / 100.0 * 0.5;
    let norm = 1.0 / (1.0 + w2);
    let amp = p.amount / 100.0 * 0.05;
    let seed = p.seed;
    let seed2 = p.seed ^ 0x9E37_79B9_7F4A_7C15;
    image
        .data_mut()
        .par_chunks_mut(w * 3)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let rgb = [row[x * 3], row[x * 3 + 1], row[x * 3 + 2]];
                let lum = luminance_rec2020(rgb);
                // Pinned: grain only for 0 < L ≤ 1 (skips non-finite, black
                // and unbounded highlights).
                if !(lum > 0.0 && lum <= 1.0) {
                    continue;
                }
                let n1 = sample_noise(x, y, cell, seed);
                let n = if w2 > 0.0 {
                    (n1 + sample_noise(x, y, half_cell, seed2) * w2) * norm
                } else {
                    n1
                };
                let l = srgb_encode(lum);
                let weight = 0.25 + 0.75 * (4.0 * l * (1.0 - l));
                let l_new = (l + n * amp * weight).clamp(0.0, 1.0);
                let lum_new = srgb_decode(l_new);
                let ratio = lum_new / lum;
                row[x * 3] = rgb[0] * ratio;
                row[x * 3 + 1] = rgb[1] * ratio;
                row[x * 3 + 2] = rgb[2] * ratio;
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: f32) -> ImageRgbF32 {
        ImageRgbF32::from_data(w, h, vec![v; w as usize * h as usize * 3])
    }

    fn vignette_only(amount: f32, midpoint: f32, roundness: f32, feather: f32) -> FinishingParams {
        FinishingParams {
            enabled: true,
            vignette: VignetteParams {
                amount,
                midpoint,
                roundness,
                feather,
            },
            grain: GrainParams::default(), // amount 0
        }
    }

    fn grain_only(amount: f32, size: f32, roughness: f32, seed: u64) -> FinishingParams {
        FinishingParams {
            enabled: true,
            vignette: VignetteParams::default(), // amount 0
            grain: GrainParams {
                amount,
                size,
                roughness,
                seed,
            },
        }
    }

    /// Standard deviation of the sRGB-encoded red channel (f64, test-only).
    fn encoded_std(img: &ImageRgbF32) -> f64 {
        let vals: Vec<f64> = img
            .data()
            .chunks(3)
            .map(|px| f64::from(srgb_encode(px[0])))
            .collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        (vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / vals.len() as f64).sqrt()
    }

    #[test]
    fn zero_amounts_and_disabled_are_bit_exact_identity() {
        let mut img = flat(16, 16, 0.5);
        img.set_pixel(3, 3, [1.9, -0.1, 0.4]);
        let orig = img.clone();
        // Defaults: vignette amount 0, grain amount 0.
        apply(&mut img, &FinishingParams::default(), 1.0);
        assert_eq!(img.data(), orig.data());
        // Disabled with both effects dialled in.
        let params = FinishingParams {
            enabled: false,
            ..grain_only(80.0, 50.0, 50.0, 7)
        };
        apply(&mut img, &params, 1.0);
        assert_eq!(img.data(), orig.data());
        let params = FinishingParams {
            enabled: false,
            ..vignette_only(-100.0, 50.0, 0.0, 50.0)
        };
        apply(&mut img, &params, 1.0);
        assert_eq!(img.data(), orig.data());
    }

    #[test]
    fn negative_vignette_darkens_corners_more_than_centre() {
        let mut img = flat(16, 16, 0.5);
        apply(&mut img, &vignette_only(-100.0, 50.0, 0.0, 50.0), 1.0);
        let corner = img.pixel(0, 0)[0];
        let centre = img.pixel(8, 8)[0];
        assert!(corner < centre, "corner {corner} vs centre {centre}");
        assert!(corner < 0.5);
        // Centre is inside the midpoint radius: falloff 0, factor 2^0 = 1.
        assert_eq!(centre, 0.5);
    }

    #[test]
    fn positive_vignette_brightens_corners() {
        let mut img = flat(16, 16, 0.5);
        apply(&mut img, &vignette_only(100.0, 50.0, 0.0, 50.0), 1.0);
        assert!(img.pixel(0, 0)[0] > 0.5);
        assert_eq!(img.pixel(8, 8)[0], 0.5);
    }

    #[test]
    fn roundness_changes_edge_versus_corner_relationship() {
        // At the edge midpoint, d_rect ≈ 0.94 but d_circ ≈ 0.66: with a
        // ramp placed between them (midpoint 80, feather 0), a rectangular
        // vignette (−100) darkens the edge midpoint while a circular one
        // (+100) leaves it untouched.
        let mut rect = flat(16, 16, 0.5);
        apply(&mut rect, &vignette_only(-100.0, 80.0, -100.0, 0.0), 1.0);
        let mut circ = flat(16, 16, 0.5);
        apply(&mut circ, &vignette_only(-100.0, 80.0, 100.0, 0.0), 1.0);
        let rect_edge = rect.pixel(15, 8)[0];
        let circ_edge = circ.pixel(15, 8)[0];
        assert!(
            rect_edge < circ_edge,
            "rectangular edge {rect_edge} must be darker than circular {circ_edge}"
        );
        // Both darken the true corner.
        assert!(rect.pixel(0, 0)[0] < 0.5);
        assert!(circ.pixel(0, 0)[0] < 0.5);
    }

    #[test]
    fn grain_is_seeded_and_deterministic() {
        let base = flat(64, 64, 0.5);
        let mut a = base.clone();
        let mut b = base.clone();
        apply(&mut a, &grain_only(50.0, 25.0, 50.0, 42), 1.0);
        apply(&mut b, &grain_only(50.0, 25.0, 50.0, 42), 1.0);
        assert_eq!(a.data(), b.data(), "same seed must be bit-identical");
        assert_ne!(a.data(), base.data(), "grain must change the image");
        let mut c = base.clone();
        apply(&mut c, &grain_only(50.0, 25.0, 50.0, 43), 1.0);
        assert_ne!(a.data(), c.data(), "different seed must differ");
    }

    #[test]
    fn grain_amplitude_scales_with_amount() {
        let base = flat(64, 64, 0.5);
        let mut lo = base.clone();
        let mut hi = base.clone();
        apply(&mut lo, &grain_only(25.0, 0.0, 0.0, 1), 1.0);
        apply(&mut hi, &grain_only(100.0, 0.0, 0.0, 1), 1.0);
        let std_lo = encoded_std(&lo);
        let std_hi = encoded_std(&hi);
        assert!(
            std_hi > std_lo * 2.0,
            "amount 100 ({std_hi}) must be much noisier than 25 ({std_lo})"
        );
    }

    #[test]
    fn midtones_are_noisier_than_extremes() {
        let run = |level: f32| {
            let mut img = flat(64, 64, level);
            apply(&mut img, &grain_only(100.0, 0.0, 0.0, 5), 1.0);
            encoded_std(&img)
        };
        let dark = run(0.001);
        let mid = run(0.5);
        let bright = run(0.98);
        assert!(mid > dark * 1.5, "mid {mid} vs dark {dark}");
        assert!(mid > bright * 1.5, "mid {mid} vs bright {bright}");
    }

    #[test]
    fn highlights_above_one_are_spared() {
        let mut img = flat(16, 16, 2.5); // unbounded highlight, L > 1
        let orig = img.clone();
        apply(&mut img, &grain_only(100.0, 25.0, 50.0, 9), 1.0);
        assert_eq!(img.data(), orig.data());
    }

    #[test]
    fn scale_changes_cell_size() {
        // size 100 → cell 4 px at scale 1, 1 px at scale 0.25. Fine grain
        // decorrelates adjacent pixels; coarse grain interpolates within
        // 4-px cells, so its adjacent-difference/std ratio is much lower.
        let ratio_for = |scale: f32| {
            let mut img = flat(64, 64, 0.5);
            apply(&mut img, &grain_only(100.0, 100.0, 0.0, 3), scale);
            let std = encoded_std(&img);
            let mut sum = 0.0_f64;
            let mut count = 0.0_f64;
            for y in 0..64 {
                for x in 0..63 {
                    sum += f64::from((img.pixel(x + 1, y)[0] - img.pixel(x, y)[0]).abs());
                    count += 1.0;
                }
            }
            (sum / count) / std
        };
        let coarse = ratio_for(1.0);
        let fine = ratio_for(0.25);
        assert!(
            fine > coarse * 1.5,
            "fine grain ({fine}) must decorrelate more than coarse ({coarse})"
        );
    }

    #[test]
    fn vignette_plus_grain_double_run_is_bit_identical() {
        let mut base = ImageRgbF32::new(48, 32);
        for y in 0..32 {
            for x in 0..48 {
                let v = 0.2 + 0.6 * (x as f32 / 47.0);
                base.set_pixel(x, y, [v, v * 0.9, v * 1.1]);
            }
        }
        let params = FinishingParams {
            enabled: true,
            vignette: VignetteParams {
                amount: -60.0,
                midpoint: 40.0,
                roundness: 30.0,
                feather: 60.0,
            },
            grain: GrainParams {
                amount: 40.0,
                size: 50.0,
                roughness: 70.0,
                seed: 1234,
            },
        };
        let mut a = base.clone();
        let mut b = base.clone();
        apply(&mut a, &params, 1.0);
        apply(&mut b, &params, 1.0);
        assert_eq!(a.data(), b.data());
        assert_ne!(a.data(), base.data());
    }
}
