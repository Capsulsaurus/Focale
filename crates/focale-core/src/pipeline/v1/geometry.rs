//! Pipeline v1 stage 9: geometry — EXIF orientation, rotate, perspective,
//! flip, crop.
//!
//! **Frozen v1 geometry model** (HARD-VER). The stage applies, in this
//! fixed order:
//!
//! 1. **EXIF orientation** (values 1–8): exact pixel shuffling
//!    (rotate 90/180/270 + flips), no resampling. Unknown values are
//!    treated as 1 (no-op).
//! 2. **Rotation + perspective** as a single inverse-warp resampling pass
//!    (homography `H = P·R`: forward rotates first, keystone second, so
//!    the inverse pass un-keystones first, then un-rotates). Skipped
//!    entirely — bit-exact — when the angle and both keystone amounts are
//!    zero. Output canvas dims equal input dims; there is **no auto
//!    zoom-to-fit in v1** (the crop rect handles framing). Out-of-frame
//!    samples are black `(0, 0, 0)`; the UI constrains the crop.
//! 3. **Horizontal flip**: exact column mirror.
//! 4. **Crop**: normalized rect → pixel rect by rounding, exact pixel copy.
//!
//! When `!params.enabled`, only the EXIF orientation applies.
//!
//! # Determinism
//!
//! Steps 1, 3 and 4 are exact integer pixel moves. Step 2 evaluates a pure
//! function of the output coordinate per pixel (`f32`, fixed expression
//! order) and is parallelized only over disjoint output rows. `sin`/`cos`
//! come from the platform maths library (see the caveat in
//! [`crate::color`]).

use rayon::prelude::*;

use crate::image::ImageRgbF32;
use crate::params::geometry::{CropRect, GeometryParams};

/// Runs the geometry stage, producing a new image (dimensions may change).
///
/// `orientation` is the EXIF orientation tag (1–8; anything else acts as
/// 1). When `!params.enabled` only the orientation is applied.
///
/// # Frozen v1 warp formulas (step 2)
///
/// On the oriented frame of size `W × H`, each **output** pixel `(x, y)`
/// is mapped to normalized centred coordinates
/// `u = (x + 0.5)/W − 0.5`, `v = (y + 0.5)/H − 0.5` (y down), then to a
/// source position by the inverse homography:
///
/// 1. Inverse keystone: with `kv = perspective.vertical / 100` and
///    `kh = perspective.horizontal / 100`,
///    `w = 1 + kv·0.4·v + kh·0.4·u`, `u_s = u/w`, `v_s = v/w`.
///    (`|kv|, |kh| ≤ 1` and `|u|, |v| ≤ 0.5` keep `w ≥ 0.6`, so the
///    division is always safe; identity at `kv = kh = 0`.)
/// 2. Inverse rotation, angle-true in aspect-corrected space with
///    `A = W/H`: `x' = u_s·A`, `y' = v_s`, and for `θ = rotate°` (content
///    rotates counter-clockwise on screen, y-down frame):
///    `x_r = cosθ·x' − sinθ·y'`, `y_r = sinθ·x' + cosθ·y'`,
///    then `u_src = x_r/A`, `v_src = y_r`.
///
/// Source pixel coordinates are `x_src = (u_src + 0.5)·W − 0.5`,
/// `y_src = (v_src + 0.5)·H − 0.5` (pixel-centre convention). **Pinned
/// out-of-frame predicate:** the sample is black when
/// `x_src < −0.5 ∨ x_src > W − 0.5 ∨ y_src < −0.5 ∨ y_src > H − 0.5`
/// (i.e. the position lies outside the image rectangle by more than half a
/// pixel from the nearest pixel centre); otherwise it is bilinearly
/// sampled with edge clamping ([`ImageRgbF32::sample_bilinear`]).
///
/// # Frozen v1 crop mapping (step 4)
///
/// On the current canvas `W × H`:
/// `x0 = round(crop.x0·W)`, `x1 = round(crop.x1·W)` (same for y with H),
/// clamped to the canvas; a degenerate rect is widened to 1 px. The output
/// is an exact pixel copy of `[x0, x1) × [y0, y1)`. `None` = full frame.
pub fn apply(image: &ImageRgbF32, params: &GeometryParams, orientation: u16) -> ImageRgbF32 {
    let oriented = apply_orientation(image, orientation);
    if !params.enabled {
        return oriented;
    }
    let needs_warp = params.rotate != 0.0
        || params.perspective.vertical != 0.0
        || params.perspective.horizontal != 0.0;
    let warped = if needs_warp {
        warp(&oriented, params)
    } else {
        oriented
    };
    let flipped = if params.flip_horizontal {
        flip_horizontal(&warped)
    } else {
        warped
    };
    match params.crop {
        Some(rect) => crop(&flipped, rect),
        None => flipped,
    }
}

/// Step 1: EXIF orientation as exact pixel shuffling.
///
/// Inverse pixel mappings (output `(x, y)` reads input `(ix, iy)`;
/// orientations 5–8 swap the canvas to `H × W`):
///
/// | tag | meaning           | `(ix, iy)`       |
/// |-----|-------------------|------------------|
/// | 1   | normal            | `(x, y)`         |
/// | 2   | mirror horizontal | `(W−1−x, y)`     |
/// | 3   | rotate 180°       | `(W−1−x, H−1−y)` |
/// | 4   | mirror vertical   | `(x, H−1−y)`     |
/// | 5   | transpose         | `(y, x)`         |
/// | 6   | rotate 90° CW     | `(y, H−1−x)`     |
/// | 7   | transverse        | `(W−1−y, H−1−x)` |
/// | 8   | rotate 270° CW    | `(W−1−y, x)`     |
fn apply_orientation(src: &ImageRgbF32, orientation: u16) -> ImageRgbF32 {
    if !(2..=8).contains(&orientation) {
        return src.clone();
    }
    let w = src.width();
    let h = src.height();
    let (ow, oh) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut out = ImageRgbF32::new(ow, oh);
    for y in 0..oh {
        for x in 0..ow {
            let (ix, iy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x), // 8
            };
            out.set_pixel(x, y, src.pixel(ix, iy));
        }
    }
    out
}

/// Step 2: the combined rotation + perspective inverse warp (formulas on
/// [`apply`]). Same canvas size as the input; parallel over disjoint
/// output rows.
fn warp(src: &ImageRgbF32, params: &GeometryParams) -> ImageRgbF32 {
    let w = src.width();
    let h = src.height();
    let wf = w as f32;
    let hf = h as f32;
    let aspect = wf / hf;
    let theta = params.rotate.to_radians();
    let cos_t = crate::math::cos(theta);
    let sin_t = crate::math::sin(theta);
    let kv = params.perspective.vertical / 100.0;
    let kh = params.perspective.horizontal / 100.0;

    let mut out = ImageRgbF32::new(w, h);
    out.data_mut()
        .par_chunks_mut(w as usize * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let v = (y as f32 + 0.5) / hf - 0.5;
            for x in 0..w as usize {
                let u = (x as f32 + 0.5) / wf - 0.5;
                // Inverse keystone (P⁻¹).
                let denom = 1.0 + kv * 0.4 * v + kh * 0.4 * u;
                let us = u / denom;
                let vs = v / denom;
                // Inverse rotation (R⁻¹) in aspect-corrected space.
                let xa = us * aspect;
                let ya = vs;
                let xr = cos_t * xa - sin_t * ya;
                let yr = sin_t * xa + cos_t * ya;
                let u_src = xr / aspect;
                let v_src = yr;
                let xs = (u_src + 0.5) * wf - 0.5;
                let ys = (v_src + 0.5) * hf - 0.5;
                let rgb = if xs < -0.5 || xs > wf - 0.5 || ys < -0.5 || ys > hf - 0.5 {
                    [0.0, 0.0, 0.0]
                } else {
                    src.sample_bilinear(xs, ys)
                };
                row[x * 3] = rgb[0];
                row[x * 3 + 1] = rgb[1];
                row[x * 3 + 2] = rgb[2];
            }
        });
    out
}

/// Step 3: exact column mirror.
fn flip_horizontal(src: &ImageRgbF32) -> ImageRgbF32 {
    let w = src.width();
    let h = src.height();
    let mut out = ImageRgbF32::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.set_pixel(x, y, src.pixel(w - 1 - x, y));
        }
    }
    out
}

/// Step 4: crop to the pinned pixel rect (mapping on [`apply`]); exact
/// pixel copy.
fn crop(src: &ImageRgbF32, rect: CropRect) -> ImageRgbF32 {
    let w = src.width() as i64;
    let h = src.height() as i64;
    let wf = src.width() as f32;
    let hf = src.height() as f32;
    let mut x0 = ((rect.x0 * wf).round() as i64).clamp(0, w);
    let mut x1 = ((rect.x1 * wf).round() as i64).clamp(0, w);
    let mut y0 = ((rect.y0 * hf).round() as i64).clamp(0, h);
    let mut y1 = ((rect.y1 * hf).round() as i64).clamp(0, h);
    if x1 <= x0 {
        x1 = (x0 + 1).min(w);
        x0 = x1 - 1;
    }
    if y1 <= y0 {
        y1 = (y0 + 1).min(h);
        y0 = y1 - 1;
    }
    let ow = (x1 - x0) as u32;
    let oh = (y1 - y0) as u32;
    let mut out = ImageRgbF32::new(ow, oh);
    for y in 0..oh {
        let src_row = src.row(y0 as u32 + y);
        let start = x0 as usize * 3;
        let end = x1 as usize * 3;
        let dst_start = y as usize * ow as usize * 3;
        out.data_mut()[dst_start..dst_start + (end - start)].copy_from_slice(&src_row[start..end]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::geometry::PerspectiveParams;

    /// 3×2 asymmetric test card; the label `k = y·3 + x` is stored in all
    /// three channels as `k/10`.
    fn card() -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(3, 2);
        for y in 0..2 {
            for x in 0..3 {
                let k = (y * 3 + x) as f32 / 10.0;
                img.set_pixel(x, y, [k, k, k]);
            }
        }
        img
    }

    fn labels(img: &ImageRgbF32) -> Vec<u32> {
        let mut out = Vec::new();
        for y in 0..img.height() {
            for x in 0..img.width() {
                out.push((img.pixel(x, y)[0] * 10.0).round() as u32);
            }
        }
        out
    }

    /// A square image with a smooth, asymmetric pattern (for resampled
    /// comparisons).
    fn smooth_square(n: u32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(n, n);
        for y in 0..n {
            for x in 0..n {
                let v = (x as f32 * 3.0 + y as f32 * 5.0) / (8.0 * n as f32);
                img.set_pixel(x, y, [v, v * 0.5, 1.0 - v]);
            }
        }
        img
    }

    fn params_rotate(rotate: f32) -> GeometryParams {
        GeometryParams {
            rotate,
            ..GeometryParams::default()
        }
    }

    #[test]
    fn orientation_permutations_are_exact() {
        // Input card:      0 1 2
        //                  3 4 5
        let img = card();
        let expected: [(u16, u32, Vec<u32>); 8] = [
            (1, 3, vec![0, 1, 2, 3, 4, 5]),
            (2, 3, vec![2, 1, 0, 5, 4, 3]),
            (3, 3, vec![5, 4, 3, 2, 1, 0]),
            (4, 3, vec![3, 4, 5, 0, 1, 2]),
            (5, 2, vec![0, 3, 1, 4, 2, 5]),
            (6, 2, vec![3, 0, 4, 1, 5, 2]),
            (7, 2, vec![5, 2, 4, 1, 3, 0]),
            (8, 2, vec![2, 5, 1, 4, 0, 3]),
        ];
        for (o, width, grid) in expected {
            let out = apply(&img, &GeometryParams::default(), o);
            assert_eq!(out.width(), width, "orientation {o} width");
            assert_eq!(labels(&out), grid, "orientation {o}");
        }
    }

    #[test]
    fn no_op_geometry_is_bit_exact_identity() {
        let img = smooth_square(8);
        let out = apply(&img, &GeometryParams::default(), 1);
        assert_eq!(out.data(), img.data());
        assert_eq!((out.width(), out.height()), (img.width(), img.height()));
    }

    #[test]
    fn disabled_applies_only_orientation() {
        let img = card();
        let params = GeometryParams {
            enabled: false,
            rotate: 45.0,
            flip_horizontal: true,
            crop: Some(CropRect {
                x0: 0.0,
                y0: 0.0,
                x1: 0.5,
                y1: 0.5,
            }),
            ..GeometryParams::default()
        };
        let out = apply(&img, &params, 3);
        assert_eq!(labels(&out), vec![5, 4, 3, 2, 1, 0]);
    }

    #[test]
    fn rotate_90_matches_orientation_8() {
        let img = smooth_square(16);
        let via_param = apply(&img, &params_rotate(90.0), 1);
        let via_orientation = apply(&img, &GeometryParams::default(), 8);
        for y in 0..16 {
            for x in 0..16 {
                let a = via_param.pixel(x, y);
                let b = via_orientation.pixel(x, y);
                for c in 0..3 {
                    assert!(
                        (a[c] - b[c]).abs() < 1e-3,
                        "mismatch at ({x},{y}) channel {c}: {} vs {}",
                        a[c],
                        b[c]
                    );
                }
            }
        }
    }

    #[test]
    fn rotate_180_matches_orientation_3() {
        let img = smooth_square(12);
        let via_param = apply(&img, &params_rotate(180.0), 1);
        let via_orientation = apply(&img, &GeometryParams::default(), 3);
        for y in 0..12 {
            for x in 0..12 {
                let a = via_param.pixel(x, y);
                let b = via_orientation.pixel(x, y);
                for c in 0..3 {
                    assert!((a[c] - b[c]).abs() < 1e-3, "mismatch at ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn flip_is_an_exact_column_mirror() {
        let img = smooth_square(8);
        // Perspective/rotate at zero skip the warp; flip must be an exact
        // column mirror.
        let params = GeometryParams {
            flip_horizontal: true,
            ..GeometryParams::default()
        };
        let out = apply(&img, &params, 1);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(out.pixel(x, y), img.pixel(7 - x, y));
            }
        }
    }

    #[test]
    fn crop_extracts_exact_pixels() {
        let mut img = ImageRgbF32::new(8, 6);
        for y in 0..6 {
            for x in 0..8 {
                let v = (y * 8 + x) as f32;
                img.set_pixel(x, y, [v, v + 0.25, v + 0.5]);
            }
        }
        let params = GeometryParams {
            crop: Some(CropRect {
                x0: 0.25,
                y0: 0.5,
                x1: 0.75,
                y1: 1.0,
            }),
            ..GeometryParams::default()
        };
        let out = apply(&img, &params, 1);
        assert_eq!((out.width(), out.height()), (4, 3));
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(out.pixel(x, y), img.pixel(x + 2, y + 3));
            }
        }
    }

    #[test]
    fn out_of_frame_corners_are_black_after_rotation() {
        let mut img = ImageRgbF32::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                img.set_pixel(x, y, [1.0, 1.0, 1.0]);
            }
        }
        let out = apply(&img, &params_rotate(45.0), 1);
        assert_eq!(out.pixel(0, 0), [0.0, 0.0, 0.0]);
        assert_eq!(out.pixel(15, 0), [0.0, 0.0, 0.0]);
        assert_eq!(out.pixel(0, 15), [0.0, 0.0, 0.0]);
        assert_eq!(out.pixel(15, 15), [0.0, 0.0, 0.0]);
        // The centre survives (within bilinear rounding of a constant
        // field, which is not exactly 1 in f32).
        for c in 0..3 {
            assert!((out.pixel(8, 8)[c] - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn full_stack_is_deterministic() {
        let img = smooth_square(16);
        let params = GeometryParams {
            rotate: 17.3,
            perspective: PerspectiveParams {
                vertical: 30.0,
                horizontal: -20.0,
            },
            flip_horizontal: true,
            crop: Some(CropRect {
                x0: 0.1,
                y0: 0.15,
                x1: 0.9,
                y1: 0.85,
            }),
            ..GeometryParams::default()
        };
        let a = apply(&img, &params, 6);
        let b = apply(&img, &params, 6);
        assert_eq!(a.data(), b.data());
        // Crop rect: x 1.6→2 .. 14.4→14, y 2.4→2 .. 13.6→14.
        assert_eq!((a.width(), a.height()), (12, 12));
    }
}
