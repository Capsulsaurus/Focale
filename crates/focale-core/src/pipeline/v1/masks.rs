//! Mask rasterization for pipeline v1 (docs/subsystems/masks.md) —
//! **frozen**: the formulas documented here define the v1 output forever.
//!
//! A [`MaskGroup`] rasterizes to a single-channel coverage plane in `[0, 1]`
//! at the context dimensions. Determinism contract (HARD-DET): all pixel
//! maths is `f32` with fixed expression order, iteration orders are fixed,
//! and `rayon` is used only over disjoint output rows (each output value
//! depends solely on its inputs, so thread count cannot change results).
//!
//! # Coordinate convention
//!
//! Mask shapes are stored in normalized coordinates `x, y ∈ [0, 1]` over the
//! working frame, y down. The sample point of output pixel `(x, y)` is the
//! pixel centre `((x + 0.5) / width, (y + 0.5) / height)`. Quantities
//! normalized "to the long edge" (brush radius, feather) scale by
//! `max(width, height)` pixels.
//!
//! # Group combination
//!
//! Coverage starts all-zero; each component is rasterized and combined in
//! order with its [`MaskOp`]: `Add → max(acc, m)`, `Subtract → acc·(1 − m)`,
//! `Intersect → acc·m`.
//!
//! # Component pipeline
//!
//! Per component: rasterize shape → invert (`m = 1 − m`) if `invert` →
//! feather (separable Gaussian blur, `σ = feather · max(width, height) / 3`)
//! → density (`m ·= density`). Inputs are sanitized deterministically:
//! `feather` clamps to `[0, 0.25]`, `density` to `[0, 1]`.

use std::io::Read;

use flate2::read::DeflateDecoder;
use rayon::prelude::*;

use crate::color::{REC2020_TO_XYZ, luminance_rec2020, srgb_encode, xyz_to_oklab};
use crate::image::{ImageGrayF32, ImageRgbF32};
use crate::masks::{
    BrushMask, BrushStroke, ColorRangeMask, LinearGradientMask, LuminanceRangeMask, MaskComponent,
    MaskGroup, MaskOp, MaskShape, RadialGradientMask, ResolvedMask,
};

/// Rasterization context: output dimensions plus the working-space image
/// (range masks sample it).
pub struct MaskContext<'a> {
    /// Target mask width (matches the working image).
    pub width: u32,
    /// Target mask height.
    pub height: u32,
    /// The working-space image at the point local adjustments apply
    /// (linear Rec.2020).
    pub image: &'a ImageRgbF32,
}

/// Rasterizes a mask group to a coverage plane in `[0, 1]` at the context
/// dimensions.
///
/// Starts from all-zero coverage and folds each component in order with its
/// [`MaskOp`] (see the module docs). An empty group is therefore all zero,
/// and a group whose first component subtracts or intersects stays zero.
pub fn rasterize_group(group: &MaskGroup, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    let mut acc = ImageGrayF32::new(ctx.width, ctx.height);
    for component in &group.components {
        let m = rasterize_component(component, ctx);
        combine(&mut acc, &m, component.op);
    }
    acc
}

/// Rasterizes one component through the full per-component pipeline
/// (shape → invert → feather → density), without applying its [`MaskOp`].
///
/// Exposed so tests and previews can inspect a single layer; the combine
/// step belongs to [`rasterize_group`].
pub fn rasterize_component(component: &MaskComponent, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    let mut m = rasterize_shape(&component.shape, ctx);
    if component.invert {
        for v in m.data_mut() {
            *v = 1.0 - *v;
        }
    }
    let long_edge = ctx.width.max(ctx.height) as f32;
    let sigma = component.feather.clamp(0.0, 0.25) * long_edge / 3.0;
    if sigma > 0.0 {
        m = gaussian_blur(&m, sigma);
    }
    let density = component.density.clamp(0.0, 1.0);
    if density < 1.0 {
        for v in m.data_mut() {
            *v *= density;
        }
    }
    m
}

/// Applies `op` element-wise: `Add → max`, `Subtract → acc·(1 − m)`,
/// `Intersect → acc·m`. Fixed row-major order.
fn combine(acc: &mut ImageGrayF32, m: &ImageGrayF32, op: MaskOp) {
    match op {
        MaskOp::Add => {
            for (a, v) in acc.data_mut().iter_mut().zip(m.data()) {
                *a = a.max(*v);
            }
        }
        MaskOp::Subtract => {
            for (a, v) in acc.data_mut().iter_mut().zip(m.data()) {
                *a *= 1.0 - *v;
            }
        }
        MaskOp::Intersect => {
            for (a, v) in acc.data_mut().iter_mut().zip(m.data()) {
                *a *= *v;
            }
        }
    }
}

fn rasterize_shape(shape: &MaskShape, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    match shape {
        MaskShape::Brush(brush) => rasterize_brush(brush, ctx.width, ctx.height),
        MaskShape::Linear(gradient) => rasterize_linear(gradient, ctx.width, ctx.height),
        MaskShape::Radial(gradient) => rasterize_radial(gradient, ctx.width, ctx.height),
        MaskShape::LuminanceRange(mask) => rasterize_luminance_range(mask, ctx),
        MaskShape::ColorRange(mask) => rasterize_color_range(mask, ctx),
        MaskShape::AiResolved(mask) => rasterize_resolved(mask, ctx.width, ctx.height),
    }
}

/// Fills a plane by evaluating `f(x, y)` per pixel, parallelized over
/// disjoint rows (deterministic: each output depends only on its own
/// coordinates and read-only captures).
fn fill_per_pixel<F>(width: u32, height: u32, f: F) -> ImageGrayF32
where
    F: Fn(u32, u32) -> f32 + Sync,
{
    let mut plane = ImageGrayF32::new(width, height);
    if width == 0 || height == 0 {
        return plane;
    }
    plane
        .data_mut()
        .par_chunks_mut(width as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = f(x as u32, y as u32);
            }
        });
    plane
}

/// Hermite smoothstep on `[edge0, edge1]`: `t = clamp((x − e0)/(e1 − e0))`,
/// result `t²(3 − 2t)`. A degenerate interval (`edge1 ≤ edge0`) is a hard
/// step: `0` for `x < edge0`, `1` otherwise.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear gradient: straight linear ramp (Adobe-style, no easing).
///
/// With `d = end − start`, the signed projection of sample point `p` is
/// `t = (p − start)·d / |d|²`; coverage is `clamp(1 − t, 0, 1)`: full on the
/// `start` side (`t ≤ 0`), zero past `end` (`t ≥ 1`). Degenerate gradient
/// (`start == end`) is an empty mask.
fn rasterize_linear(gradient: &LinearGradientMask, width: u32, height: u32) -> ImageGrayF32 {
    let dx = gradient.end[0] - gradient.start[0];
    let dy = gradient.end[1] - gradient.start[1];
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return ImageGrayF32::new(width, height);
    }
    let inv_w = 1.0 / width as f32;
    let inv_h = 1.0 / height as f32;
    fill_per_pixel(width, height, |x, y| {
        let px = (x as f32 + 0.5) * inv_w;
        let py = (y as f32 + 0.5) * inv_h;
        let t = ((px - gradient.start[0]) * dx + (py - gradient.start[1]) * dy) / len2;
        (1.0 - t).clamp(0.0, 1.0)
    })
}

/// Radial (elliptical) gradient.
///
/// The sample point is rotated by `−rotation` about `center` in the
/// normalized (y-down) frame — with `θ = rotation·π/180`,
/// `ex = cosθ·dx + sinθ·dy`, `ey = −sinθ·dx + cosθ·dy` — then the
/// normalized ellipse distance is `e = √((ex/rx)² + (ey/ry)²)`. Coverage is
/// `1 − smoothstep(1 − falloff, 1, e)`: full inside `e ≤ 1 − falloff`, zero
/// at `e ≥ 1` (`falloff = 0` gives a hard edge, inside inclusive). All
/// maths is in normalized coordinates, so on non-square images the ellipse
/// covers `radius·dimension` pixels per axis. A non-positive semi-axis is
/// an empty mask.
fn rasterize_radial(gradient: &RadialGradientMask, width: u32, height: u32) -> ImageGrayF32 {
    let rx = gradient.radius[0];
    let ry = gradient.radius[1];
    if rx <= 0.0 || ry <= 0.0 {
        return ImageGrayF32::new(width, height);
    }
    let (sin, cos) = gradient.rotation.to_radians().sin_cos();
    let inner = 1.0 - gradient.falloff.clamp(0.0, 1.0);
    let inv_w = 1.0 / width as f32;
    let inv_h = 1.0 / height as f32;
    fill_per_pixel(width, height, |x, y| {
        let dx = (x as f32 + 0.5) * inv_w - gradient.center[0];
        let dy = (y as f32 + 0.5) * inv_h - gradient.center[1];
        let ex = (cos * dx + sin * dy) / rx;
        let ey = (-sin * dx + cos * dy) / ry;
        let e = (ex * ex + ey * ey).sqrt();
        1.0 - smoothstep(inner, 1.0, e)
    })
}

/// Luminance range: band over a perceptual luminance axis.
///
/// Per pixel, `Y = luminance_rec2020(rgb)` (linear, working space), mapped
/// to the display-referred axis `y = srgb_encode(clamp(Y, 0, 1))` so the
/// `low`/`high` bounds are perceptually spaced. With ramp half-width
/// `f = falloff·0.5`, coverage is
/// `smoothstep(low − f, low, y) · (1 − smoothstep(high, high + f, y))`:
/// `1` inside `[low, high]`, easing to `0` over `f` beyond each edge,
/// `0` outside. `falloff = 0` gives hard, inclusive band edges.
fn rasterize_luminance_range(mask: &LuminanceRangeMask, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    debug_assert_eq!(ctx.image.width(), ctx.width);
    debug_assert_eq!(ctx.image.height(), ctx.height);
    let half = 0.5 * mask.falloff.clamp(0.0, 1.0);
    let image = ctx.image;
    fill_per_pixel(ctx.width, ctx.height, |x, y| {
        let signal = srgb_encode(luminance_rec2020(image.pixel(x, y)));
        let rise = if half > 0.0 {
            smoothstep(mask.low - half, mask.low, signal)
        } else if signal >= mask.low {
            1.0
        } else {
            0.0
        };
        let fall = if half > 0.0 {
            1.0 - smoothstep(mask.high, mask.high + half, signal)
        } else if signal <= mask.high {
            1.0
        } else {
            0.0
        };
        rise * fall
    })
}

/// Colour range: coverage from Oklab distance to the sampled colour.
///
/// Both the reference colour and each pixel (linear Rec.2020) are converted
/// via `XYZ` ([`REC2020_TO_XYZ`]) to Oklab; `d` is the Euclidean distance in
/// Oklab. Coverage is `1 − smoothstep(tolerance·(1 − falloff), tolerance, d)`:
/// full for `d ≤ tolerance·(1 − falloff)`, easing to zero at `d = tolerance`.
/// `tolerance ≤ 0` selects nothing.
fn rasterize_color_range(mask: &ColorRangeMask, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    debug_assert_eq!(ctx.image.width(), ctx.width);
    debug_assert_eq!(ctx.image.height(), ctx.height);
    let tolerance = mask.tolerance.clamp(0.0, 1.0);
    if tolerance <= 0.0 {
        return ImageGrayF32::new(ctx.width, ctx.height);
    }
    let inner = tolerance * (1.0 - mask.falloff.clamp(0.0, 1.0));
    let reference = xyz_to_oklab(REC2020_TO_XYZ.mul_vec(mask.color));
    let image = ctx.image;
    fill_per_pixel(ctx.width, ctx.height, |x, y| {
        let lab = xyz_to_oklab(REC2020_TO_XYZ.mul_vec(image.pixel(x, y)));
        let dl = lab[0] - reference[0];
        let da = lab[1] - reference[1];
        let db = lab[2] - reference[2];
        let d = (dl * dl + da * da + db * db).sqrt();
        1.0 - smoothstep(inner, tolerance, d)
    })
}

/// Brush: strokes applied in order, each stamped along its polyline.
///
/// Stamp maths happens in a continuous pixel frame where pixel `(x, y)`'s
/// centre is `(x + 0.5, y + 0.5)` and a normalized point maps to
/// `(nx·width, ny·height)`. Per stroke, `radius_px = radius·max(w, h)`;
/// stamps are placed by deterministic arc-length marching: one stamp at
/// `points[0]`, then every `spacing = max(radius_px·0.25, 1)` pixels along
/// the polyline, carrying the residual distance across segments
/// (zero-length segments are skipped; a single point yields one stamp).
///
/// Stamp profile: `1` for distance `d ≤ radius_px·(1 − feather)`, Hermite
/// smoothstep falloff to `0` at `radius_px` (hard-edged for `feather = 0`).
/// Paint strokes build up flow per stamp, `m ← m + flow·profile·(1 − m)`;
/// eraser strokes remove it, `m ← m·(1 − flow·profile)` — both against the
/// brush mask accumulated so far, in stroke then stamp order. `flow` and
/// `feather` clamp to `[0, 1]`; a non-positive radius stroke is skipped.
fn rasterize_brush(brush: &BrushMask, width: u32, height: u32) -> ImageGrayF32 {
    let mut m = ImageGrayF32::new(width, height);
    if width == 0 || height == 0 {
        return m;
    }
    let long_edge = width.max(height) as f32;
    for stroke in &brush.strokes {
        apply_stroke(&mut m, stroke, long_edge);
    }
    m
}

/// Marches one stroke's polyline by arc length, stamping into `m`.
fn apply_stroke(m: &mut ImageGrayF32, stroke: &BrushStroke, long_edge: f32) {
    let radius = stroke.radius * long_edge;
    if radius.is_nan() || radius <= 0.0 || stroke.points.is_empty() {
        return;
    }
    let flow = stroke.flow.clamp(0.0, 1.0);
    let inner = radius * (1.0 - stroke.feather.clamp(0.0, 1.0));
    let spacing = (radius * 0.25).max(1.0);
    let w = m.width() as f32;
    let h = m.height() as f32;
    let to_px = |p: &[f32; 2]| [p[0] * w, p[1] * h];

    stamp(
        m,
        to_px(&stroke.points[0]),
        radius,
        inner,
        flow,
        stroke.erase,
    );
    // Distance along the remaining polyline to the next stamp.
    let mut next = spacing;
    for pair in stroke.points.windows(2) {
        let a = to_px(&pair[0]);
        let b = to_px(&pair[1]);
        let sx = b[0] - a[0];
        let sy = b[1] - a[1];
        let len = (sx * sx + sy * sy).sqrt();
        if len.is_nan() || len <= 0.0 {
            continue;
        }
        while next <= len {
            let t = next / len;
            stamp(
                m,
                [a[0] + sx * t, a[1] + sy * t],
                radius,
                inner,
                flow,
                stroke.erase,
            );
            next += spacing;
        }
        next -= len;
    }
}

/// One brush stamp at pixel-frame centre `c` (see [`rasterize_brush`]).
fn stamp(m: &mut ImageGrayF32, c: [f32; 2], radius: f32, inner: f32, flow: f32, erase: bool) {
    let width = m.width();
    let height = m.height();
    let x_lo = (c[0] - radius - 0.5).floor();
    let x_hi = (c[0] + radius - 0.5).ceil();
    let y_lo = (c[1] - radius - 0.5).floor();
    let y_hi = (c[1] + radius - 0.5).ceil();
    if x_hi < 0.0 || y_hi < 0.0 || x_lo >= width as f32 || y_lo >= height as f32 {
        return;
    }
    let x0 = x_lo.max(0.0) as u32;
    let x1 = x_hi.min((width - 1) as f32) as u32;
    let y0 = y_lo.max(0.0) as u32;
    let y1 = y_hi.min((height - 1) as f32) as u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5) - c[0];
            let dy = (y as f32 + 0.5) - c[1];
            let d = (dx * dx + dy * dy).sqrt();
            let profile = 1.0 - smoothstep(inner, radius, d);
            if profile <= 0.0 {
                continue;
            }
            let v = m.get(x, y);
            let out = if erase {
                v * (1.0 - flow * profile)
            } else {
                v + flow * profile * (1.0 - v)
            };
            m.set(x, y, out);
        }
    }
}

/// AI-resolved mask: inflate, normalize, bilinear-resample to context dims.
///
/// `deflate_bitmap` is a raw DEFLATE stream (RFC 1951, no zlib wrapper) of
/// `width × height` row-major bytes; values map to coverage as `byte/255`.
/// The plane is then resampled with [`ImageGrayF32::sample_bilinear`] under
/// the pixel-centre mapping `src = (dst + 0.5)·src_dim/dst_dim − 0.5`
/// (edge-clamped; the identity when dimensions match). A malformed stream
/// or size mismatch yields an empty (all-zero) mask — deterministic, never
/// a panic on the export path.
fn rasterize_resolved(mask: &ResolvedMask, width: u32, height: u32) -> ImageGrayF32 {
    if mask.width == 0 || mask.height == 0 || width == 0 || height == 0 {
        return ImageGrayF32::new(width, height);
    }
    let expected = mask.width as usize * mask.height as usize;
    let mut bytes = Vec::with_capacity(expected);
    let mut decoder = DeflateDecoder::new(mask.deflate_bitmap.as_slice());
    if decoder.read_to_end(&mut bytes).is_err() || bytes.len() != expected {
        tracing::warn!(
            width = mask.width,
            height = mask.height,
            got = bytes.len(),
            "resolved mask bitmap failed to inflate; treating as empty"
        );
        return ImageGrayF32::new(width, height);
    }
    let data: Vec<f32> = bytes.iter().map(|&b| b as f32 / 255.0).collect();
    let src = ImageGrayF32::from_data(mask.width, mask.height, data);
    let sx = mask.width as f32 / width as f32;
    let sy = mask.height as f32 / height as f32;
    fill_per_pixel(width, height, |x, y| {
        src.sample_bilinear((x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5)
    })
}

/// Separable Gaussian blur with clamp-to-edge sampling (the feather kernel).
///
/// Kernel: radius `r = clamp(ceil(3σ), 1, 512)`, weights
/// `w_i = exp(−i²/(2σ²))` for `i ∈ 0..=r` computed in `f32`, normalized by
/// `w₀ + 2·Σ w_i` accumulated in index order. Horizontal pass then vertical
/// pass; per output sample the accumulation is
/// `acc = w₀·centre; acc += w_i·(left_i + right_i)` for `i = 1..=r` — a
/// fixed order, so results are bit-identical regardless of thread count
/// (rows are computed independently).
fn gaussian_blur(src: &ImageGrayF32, sigma: f32) -> ImageGrayF32 {
    let width = src.width() as usize;
    let height = src.height() as usize;
    if width == 0 || height == 0 {
        return src.clone();
    }
    let radius = ((3.0 * sigma).ceil() as usize).clamp(1, 512);
    let inv_two_sigma2 = 1.0 / (2.0 * sigma * sigma);
    let mut kernel = Vec::with_capacity(radius + 1);
    for i in 0..=radius {
        let fi = i as f32;
        kernel.push(crate::math::exp(-(fi * fi) * inv_two_sigma2));
    }
    let mut sum = kernel[0];
    for k in kernel.iter().skip(1) {
        sum += 2.0 * k;
    }
    let inv_sum = 1.0 / sum;
    for k in &mut kernel {
        *k *= inv_sum;
    }

    // Horizontal pass: each output row reads only its own source row.
    let mut tmp = ImageGrayF32::new(src.width(), src.height());
    let src_data = src.data();
    tmp.data_mut()
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            let src_row = &src_data[y * width..y * width + width];
            for (x, out) in row.iter_mut().enumerate() {
                let mut acc = kernel[0] * src_row[x];
                for (i, k) in kernel.iter().enumerate().skip(1) {
                    let left = src_row[x.saturating_sub(i)];
                    let right = src_row[(x + i).min(width - 1)];
                    acc += k * (left + right);
                }
                *out = acc;
            }
        });

    // Vertical pass: rows read the (now immutable) horizontal result.
    let mut out = ImageGrayF32::new(src.width(), src.height());
    let tmp_data = tmp.data();
    out.data_mut()
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = kernel[0] * tmp_data[y * width + x];
                for (i, k) in kernel.iter().enumerate().skip(1) {
                    let up = tmp_data[y.saturating_sub(i) * width + x];
                    let down = tmp_data[(y + i).min(height - 1) * width + x];
                    acc += k * (up + down);
                }
                *o = acc;
            }
        });
    out
}
