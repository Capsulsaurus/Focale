//! Pipeline v1 stage 8: retouch — clone and heal stamps.
//!
//! **Frozen v1 algorithm** (HARD-VER). Heal is a *mean-matched clone*
//! (per-channel destination/source mean ratio), not a Poisson blend —
//! content-aware retouch is out of scope for v1 (architecture.md §3 stage 8).
//!
//! # Determinism
//!
//! Strokes are applied strictly in sidecar order, and stamps within a
//! stroke strictly along the path; each stamp gathers all of its source
//! samples and statistics *before* writing, then writes its disk in scan
//! order, so later stamps see earlier results and no read is affected by
//! the write order within a stamp. The whole stage is single-threaded —
//! stamps are small and inherently ordered.

use crate::image::ImageRgbF32;
use crate::params::retouch::{RetouchMode, RetouchParams, RetouchStroke};

/// Applies all retouch strokes in order, in place.
///
/// Skipped (bit-exact identity) when `!params.enabled` or there are no
/// strokes.
///
/// `_scale` is accepted for stage-signature uniformity but unused: stroke
/// coordinates and radii are normalized to the image (radius is a fraction
/// of the long edge), so converting them against the *current* pixel
/// dimensions is automatically preview/export-consistent — no explicit
/// scale factor is needed.
///
/// # Frozen v1 geometry
///
/// - Normalized point `p` maps to pixel-centre coordinates
///   `(p[0]·W − 0.5, p[1]·H − 0.5)` (integer coordinates are pixel
///   centres, matching [`ImageRgbF32::sample_bilinear`]).
/// - `radius_px = stroke.radius · max(W, H)`.
/// - Source offset in pixels:
///   `(source_offset[0]·W, source_offset[1]·H)` — the offset lives in the
///   same normalized frame as the destination points.
/// - Stamp centres walk the destination polyline by arc length (segment
///   length = `sqrt(dx² + dy²)`) with spacing
///   `max(radius_px · 0.5, 1.0)`: a stamp at distance 0, one at every
///   further multiple of the spacing, and a final stamp at the end point
///   when it is more than 10⁻³ px from the last emitted centre. A
///   single-point destination produces exactly one stamp.
///
/// # Frozen v1 stamp formula
///
/// For each pixel whose centre lies at distance `d < radius_px` from the
/// stamp centre, the profile is
/// `p = 1` for `d ≤ r₀`, `p = 1 − smoothstep(r₀, radius_px, d)` otherwise,
/// with `r₀ = radius_px · (1 − feather)` (feather clamped to [0, 1]).
/// The source value `src` is sampled bilinearly at `pixel + offset`
/// (edge-clamped by [`ImageRgbF32::sample_bilinear`]).
///
/// - **Clone:** `dst ← dst + (src − dst) · (p · opacity)`.
/// - **Heal** (mean-matched clone): with per-channel means `μ_s` (source
///   samples) and `μ_d` (destination content *before this stamp*), both
///   over the pixels with `p > 0.5` in scan order, and
///   `r_c = μ_d / μ_s` (forced to 1 when `|μ_s| < 10⁻⁸` or the disk has no
///   `p > 0.5` pixels): `dst ← dst + (src · r_c − dst) · (p · opacity)`.
pub fn apply(image: &mut ImageRgbF32, params: &RetouchParams, _scale: f32) {
    if !params.enabled || params.strokes.is_empty() {
        return;
    }
    let w = image.width();
    let h = image.height();
    if w == 0 || h == 0 {
        return;
    }
    let wf = w as f32;
    let hf = h as f32;
    let long_edge = wf.max(hf);

    for stroke in &params.strokes {
        let radius_px = stroke.radius * long_edge;
        if radius_px.is_nan() || radius_px <= 0.0 || stroke.dest.is_empty() {
            continue;
        }
        let opacity = stroke.opacity.clamp(0.0, 1.0);
        if opacity == 0.0 {
            continue;
        }
        let offset = (stroke.source_offset[0] * wf, stroke.source_offset[1] * hf);
        for centre in stamp_centres(stroke, wf, hf, radius_px) {
            apply_stamp(
                image,
                stroke.mode,
                centre,
                offset,
                radius_px,
                stroke.feather,
                opacity,
            );
        }
    }
}

/// Hermite smoothstep: 0 for `x ≤ e0`, 1 for `x ≥ e1`, else `t²(3 − 2t)`
/// with `t = (x − e0)/(e1 − e0)`. Callers guarantee `e1 > e0`.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Stamp opacity profile: 1 inside `radius·(1−feather)`, smoothstep down
/// to 0 at `radius` (see [`apply`]).
fn stamp_profile(d: f32, radius: f32, feather: f32) -> f32 {
    let r0 = radius * (1.0 - feather.clamp(0.0, 1.0));
    if d <= r0 {
        1.0
    } else if d >= radius {
        0.0
    } else {
        1.0 - smoothstep(r0, radius, d)
    }
}

/// Stamp centres along the destination polyline (pinned walk, see
/// [`apply`]). Returned in path order.
fn stamp_centres(stroke: &RetouchStroke, wf: f32, hf: f32, radius_px: f32) -> Vec<(f32, f32)> {
    let pts: Vec<(f32, f32)> = stroke
        .dest
        .iter()
        .map(|p| (p[0] * wf - 0.5, p[1] * hf - 0.5))
        .collect();
    if pts.len() == 1 {
        return pts;
    }
    let spacing = (radius_px * 0.5).max(1.0);
    let mut out = vec![pts[0]];
    let mut travelled = 0.0_f32;
    let mut next_d = spacing;
    for i in 1..pts.len() {
        let (x0, y0) = pts[i - 1];
        let (x1, y1) = pts[i];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let seg = (dx * dx + dy * dy).sqrt();
        if seg.is_nan() || seg <= 0.0 {
            continue;
        }
        while next_d <= travelled + seg {
            let t = (next_d - travelled) / seg;
            out.push((x0 + dx * t, y0 + dy * t));
            next_d += spacing;
        }
        travelled += seg;
    }
    let last = pts[pts.len() - 1];
    let emitted = out[out.len() - 1];
    let ex = last.0 - emitted.0;
    let ey = last.1 - emitted.1;
    if (ex * ex + ey * ey).sqrt() > 1e-3 {
        out.push(last);
    }
    out
}

/// Applies one stamp: gather pass (source samples + heal means, scan
/// order), then write pass (scan order).
fn apply_stamp(
    image: &mut ImageRgbF32,
    mode: RetouchMode,
    centre: (f32, f32),
    offset: (f32, f32),
    radius: f32,
    feather: f32,
    opacity: f32,
) {
    let (cx, cy) = centre;
    let w = image.width() as i64;
    let h = image.height() as i64;
    let x0 = ((cx - radius).floor() as i64).max(0);
    let x1 = ((cx + radius).ceil() as i64).min(w - 1);
    let y0 = ((cy - radius).floor() as i64).max(0);
    let y1 = ((cy + radius).ceil() as i64).min(h - 1);
    if x1 < x0 || y1 < y0 {
        return;
    }

    // Gather pass: profile + bilinear source sample per disk pixel, plus
    // sequential per-channel sums over p > 0.5 pixels for heal.
    let mut samples: Vec<(u32, u32, f32, [f32; 3])> = Vec::new();
    let mut sum_src = [0.0_f32; 3];
    let mut sum_dst = [0.0_f32; 3];
    let mut count = 0_u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d >= radius {
                continue;
            }
            let p = stamp_profile(d, radius, feather);
            if p <= 0.0 {
                continue;
            }
            let src = image.sample_bilinear(x as f32 + offset.0, y as f32 + offset.1);
            if p > 0.5 {
                let dst = image.pixel(x as u32, y as u32);
                for c in 0..3 {
                    sum_src[c] += src[c];
                    sum_dst[c] += dst[c];
                }
                count += 1;
            }
            samples.push((x as u32, y as u32, p, src));
        }
    }

    let ratio = match mode {
        RetouchMode::Clone => [1.0_f32; 3],
        RetouchMode::Heal => {
            let mut r = [1.0_f32; 3];
            if count > 0 {
                let n = count as f32;
                for c in 0..3 {
                    let mu_s = sum_src[c] / n;
                    let mu_d = sum_dst[c] / n;
                    if mu_s.abs() >= 1e-8 {
                        r[c] = mu_d / mu_s;
                    }
                }
            }
            r
        }
    };

    // Write pass, scan order.
    for (x, y, p, src) in samples {
        let dst = image.pixel(x, y);
        let a = p * opacity;
        let mut out = [0.0_f32; 3];
        for c in 0..3 {
            let matched = src[c] * ratio[c];
            out[c] = dst[c] + (matched - dst[c]) * a;
        }
        image.set_pixel(x, y, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, rgb: [f32; 3]) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_pixel(x, y, rgb);
            }
        }
        img
    }

    fn paint(img: &mut ImageRgbF32, x0: u32, y0: u32, x1: u32, y1: u32, rgb: [f32; 3]) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                img.set_pixel(x, y, rgb);
            }
        }
    }

    fn stroke(
        mode: RetouchMode,
        radius: f32,
        feather: f32,
        opacity: f32,
        dest: Vec<[f32; 2]>,
        source_offset: [f32; 2],
    ) -> RetouchParams {
        RetouchParams {
            enabled: true,
            strokes: vec![RetouchStroke {
                mode,
                radius,
                feather,
                opacity,
                dest,
                source_offset,
            }],
        }
    }

    fn assert_close(actual: f32, expected: f32, tol: f32) {
        assert!(
            (actual - expected).abs() <= tol,
            "{actual} differs from {expected} by more than {tol}"
        );
    }

    #[test]
    fn disabled_or_empty_is_bit_exact_identity() {
        let mut img = flat(8, 8, [0.4, 0.4, 0.4]);
        img.set_pixel(2, 3, [0.9, 0.1, 0.2]);
        let orig = img.clone();
        // No strokes.
        apply(&mut img, &RetouchParams::default(), 1.0);
        assert_eq!(img.data(), orig.data());
        // Disabled with a stroke present.
        let params = RetouchParams {
            enabled: false,
            ..stroke(
                RetouchMode::Clone,
                0.2,
                0.0,
                1.0,
                vec![[0.5, 0.5]],
                [0.2, 0.0],
            )
        };
        apply(&mut img, &params, 1.0);
        assert_eq!(img.data(), orig.data());
    }

    #[test]
    fn clone_copies_a_distinctive_patch() {
        let mut img = flat(32, 32, [0.4, 0.4, 0.4]);
        paint(&mut img, 18, 18, 30, 30, [0.8, 0.1, 0.3]);
        // Dest centre (7.5, 7.5); source offset +16 px in x and y.
        let params = stroke(
            RetouchMode::Clone,
            0.15,
            0.0,
            1.0,
            vec![[0.25, 0.25]],
            [0.5, 0.5],
        );
        apply(&mut img, &params, 1.0);
        // Inside the stamp: the source colour (within f32 rounding of the
        // lerp `dst + (src − dst)·1`).
        for (c, expected) in [0.8, 0.1, 0.3].into_iter().enumerate() {
            assert_close(img.pixel(7, 7)[c], expected, 1e-6);
            assert_close(img.pixel(8, 8)[c], expected, 1e-6);
        }
        // Outside the stamp radius (4.8 px): untouched.
        assert_eq!(img.pixel(1, 7), [0.4, 0.4, 0.4]);
        assert_eq!(img.pixel(0, 0), [0.4, 0.4, 0.4]);
    }

    #[test]
    fn heal_matches_destination_mean() {
        // Source texture is flat 0.2, destination is flat 0.4: a heal must
        // land at the destination level (ratio 0.4/0.2 = 2 per channel).
        let mut img = flat(32, 32, [0.4, 0.4, 0.4]);
        paint(&mut img, 16, 16, 31, 31, [0.2, 0.2, 0.2]);
        let params = stroke(
            RetouchMode::Heal,
            0.15,
            0.0,
            1.0,
            vec![[0.25, 0.25]],
            [0.5, 0.5],
        );
        apply(&mut img, &params, 1.0);
        for c in 0..3 {
            assert_close(img.pixel(7, 7)[c], 0.4, 1e-4);
        }
    }

    #[test]
    fn opacity_blends() {
        let mut img = flat(32, 32, [0.4, 0.4, 0.4]);
        paint(&mut img, 16, 16, 31, 31, [0.8, 0.8, 0.8]);
        let params = stroke(
            RetouchMode::Clone,
            0.15,
            0.0,
            0.5,
            vec![[0.25, 0.25]],
            [0.5, 0.5],
        );
        apply(&mut img, &params, 1.0);
        assert_close(img.pixel(7, 7)[0], 0.6, 1e-5);
    }

    #[test]
    fn feather_weakens_the_rim() {
        let mut img = flat(32, 32, [0.4, 0.4, 0.4]);
        paint(&mut img, 14, 14, 31, 31, [0.8, 0.8, 0.8]);
        let params = stroke(
            RetouchMode::Clone,
            0.15, // 4.8 px
            0.8,
            1.0,
            vec![[0.25, 0.25]],
            [0.5, 0.5],
        );
        apply(&mut img, &params, 1.0);
        let centre_effect = (img.pixel(7, 7)[0] - 0.4).abs();
        let rim_effect = (img.pixel(11, 7)[0] - 0.4).abs();
        assert!(
            centre_effect > rim_effect,
            "{centre_effect} vs {rim_effect}"
        );
        assert!(rim_effect > 0.0, "feathered rim must still receive paint");
        assert_close(img.pixel(7, 7)[0], 0.8, 1e-5); // core is at full profile
    }

    #[test]
    fn dragged_stroke_covers_the_path_without_gaps() {
        let (w, h) = (64u32, 32u32);
        let mut img = flat(w, h, [0.1, 0.1, 0.1]);
        // Source rows near the bottom, flat 0.9; offset (0, +12 px).
        paint(&mut img, 0, 24, 63, 31, [0.9, 0.9, 0.9]);
        let params = stroke(
            RetouchMode::Clone,
            0.05, // 3.2 px on the 64-px long edge
            0.0,
            1.0,
            vec![[0.2, 0.5], [0.6, 0.5]],
            [0.0, 0.375],
        );
        apply(&mut img, &params, 1.0);
        // Path runs at y = 15.5 from x = 12.3 to 37.9; every pixel along
        // row 15 between the endpoints must be fully painted (spacing =
        // radius/2 guarantees coverage).
        for x in 13..=37 {
            assert_close(img.pixel(x, 15)[0], 0.9, 1e-5);
        }
    }

    #[test]
    fn double_run_is_bit_identical() {
        let mut img = flat(48, 48, [0.3, 0.35, 0.4]);
        paint(&mut img, 30, 30, 47, 47, [0.7, 0.6, 0.5]);
        paint(&mut img, 5, 5, 12, 12, [0.05, 0.1, 0.15]);
        let params = RetouchParams {
            enabled: true,
            strokes: vec![
                RetouchStroke {
                    mode: RetouchMode::Heal,
                    radius: 0.08,
                    feather: 0.5,
                    opacity: 0.9,
                    dest: vec![[0.2, 0.2], [0.5, 0.4], [0.7, 0.3]],
                    source_offset: [0.25, 0.35],
                },
                RetouchStroke {
                    mode: RetouchMode::Clone,
                    radius: 0.1,
                    feather: 0.3,
                    opacity: 0.7,
                    dest: vec![[0.6, 0.6]],
                    source_offset: [-0.3, -0.2],
                },
            ],
        };
        let mut a = img.clone();
        let mut b = img.clone();
        apply(&mut a, &params, 1.0);
        apply(&mut b, &params, 1.0);
        assert_eq!(a.data(), b.data());
        // And the strokes actually did something.
        assert_ne!(a.data(), img.data());
    }
}
