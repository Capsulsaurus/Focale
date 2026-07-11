//! Model preprocessing and mask resolution.
//!
//! Everything here is plain CPU math with fixed iteration order. It does not
//! need to be bit-identical across machines (mask *creation* is interactive;
//! only the stored [`ResolvedMask`] participates in the determinism
//! guarantee), but it is deterministic on a given machine anyway.
//!
//! # Colour handling
//!
//! The working image is linear Rec.2020 (PRD §3). Every model in this crate
//! was trained on sRGB-encoded photographs, so preprocessing converts:
//! resize in linear light first (bilinear, pixel-centre convention), then
//! per pixel `Rec.2020 → sRGB` via [`REC2020_TO_SRGB`] and the IEC 61966-2-1
//! transfer curve ([`srgb_encode`]), clamped to [0, 1].
//!
//! # Per-model constants
//!
//! - **MobileSAM encoder**: longest side resized to
//!   [`SAM_INPUT_SIZE`] = 1024 (aspect preserved, SAM's `ResizeLongestSide`
//!   rounding `round(side · scale)`), values scaled to [0, 255], layout
//!   HWC `[h, w, 3]`. Normalization with mean [`SAM_PIXEL_MEAN`] =
//!   `[123.675, 116.28, 103.53]` and std [`SAM_PIXEL_STD`] =
//!   `[58.395, 57.12, 57.375]` plus bottom/right zero-padding to 1024×1024
//!   happens *inside* the exported encoder graph (`Acly/MobileSAM` export
//!   with `use_preprocess=True`), so the caller must NOT normalize.
//! - **BiSeNet face parsing**: squash-resized to
//!   [`FACE_PARSING_SIZE`]² = 512×512 (no letterbox, matching the
//!   `yakhyo/face-parsing` reference inference), sRGB in [0, 1], normalized
//!   with ImageNet statistics [`IMAGENET_MEAN`] = `[0.485, 0.456, 0.406]`
//!   and [`IMAGENET_STD`] = `[0.229, 0.224, 0.225]`, layout CHW
//!   `[1, 3, 512, 512]`.
//! - **U²-Net (saliency and sky)**: squash-resized to
//!   [`U2NET_INPUT_SIZE`]² = 320×320, sRGB in [0, 1], ImageNet
//!   normalization as above (the rembg preprocessing), layout CHW
//!   `[1, 3, 320, 320]`.

use flate2::Compression;
use flate2::write::DeflateEncoder;
use focale_core::color::primaries::REC2020_TO_SRGB;
use focale_core::color::transfer::srgb_encode;
use focale_core::image::{ImageGrayF32, ImageRgbF32};
use focale_core::masks::{ResolvedMask, SegmentKind};
use std::io::Write;

/// SAM input resolution: the longest image side is resized to this.
pub const SAM_INPUT_SIZE: u32 = 1024;
/// SAM pixel mean (0–255 RGB) — applied *inside* the encoder graph.
pub const SAM_PIXEL_MEAN: [f32; 3] = [123.675, 116.28, 103.53];
/// SAM pixel std (0–255 RGB) — applied *inside* the encoder graph.
pub const SAM_PIXEL_STD: [f32; 3] = [58.395, 57.12, 57.375];
/// BiSeNet face-parsing input is 512×512.
pub const FACE_PARSING_SIZE: u32 = 512;
/// U²-Net input is 320×320.
pub const U2NET_INPUT_SIZE: u32 = 320;
/// ImageNet channel means on [0, 1] sRGB (face parsing + U²-Net).
pub const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// ImageNet channel stds on [0, 1] sRGB (face parsing + U²-Net).
pub const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Linear Rec.2020 → sRGB-encoded [0, 1] (matrix, clamp, transfer curve).
pub(crate) fn linear_rec2020_to_srgb01(rgb: [f32; 3]) -> [f32; 3] {
    let lin = REC2020_TO_SRGB.mul_vec(rgb);
    [
        srgb_encode(lin[0]),
        srgb_encode(lin[1]),
        srgb_encode(lin[2]),
    ]
}

/// Resizes the working image to `tw × th` in linear light (bilinear,
/// pixel-centre mapping `src = (dst + 0.5)·src/dst − 0.5`, edge-clamped),
/// then converts each pixel to sRGB-encoded [0, 1]. Interleaved RGB output.
pub(crate) fn resample_to_srgb(image: &ImageRgbF32, tw: u32, th: u32) -> Vec<f32> {
    let sx = image.width() as f32 / tw as f32;
    let sy = image.height() as f32 / th as f32;
    let mut out = Vec::with_capacity(tw as usize * th as usize * 3);
    for y in 0..th {
        for x in 0..tw {
            let lin =
                image.sample_bilinear((x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5);
            let srgb = linear_rec2020_to_srgb01(lin);
            out.extend_from_slice(&srgb);
        }
    }
    out
}

/// Interleaved sRGB [0, 1] → planar CHW normalized `(v − mean) / std`.
pub(crate) fn chw_normalized(srgb: &[f32], mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
    let pixels = srgb.len() / 3;
    let mut out = vec![0.0f32; srgb.len()];
    for (i, px) in srgb.chunks_exact(3).enumerate() {
        for c in 0..3 {
            out[c * pixels + i] = (px[c] - mean[c]) / std[c];
        }
    }
    out
}

/// SAM's `ResizeLongestSide`: scales so the longest side becomes
/// [`SAM_INPUT_SIZE`], rounding the other side to the nearest pixel
/// (`round(side · scale)`, minimum 1).
pub(crate) fn sam_scaled_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h) as f32;
    let scale = SAM_INPUT_SIZE as f32 / long;
    let sw = ((w as f32 * scale).round() as u32).max(1);
    let sh = ((h as f32 * scale).round() as u32).max(1);
    (sw, sh)
}

/// Half of the working resolution (rounded up, minimum 1) — the storage
/// resolution for resolved AI masks (docs/architecture.md §6).
pub(crate) fn half_dims(w: u32, h: u32) -> (u32, u32) {
    (w.div_ceil(2).max(1), h.div_ceil(2).max(1))
}

/// Bilinearly resamples a coverage plane from `sw × sh` to `tw × th`
/// (pixel-centre mapping, edge-clamped — the same convention the export
/// rasterizer uses when upsampling the stored bitmap).
pub(crate) fn resample_coverage(src: &[f32], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<f32> {
    let plane = ImageGrayF32::from_data(sw, sh, src.to_vec());
    let sx = sw as f32 / tw as f32;
    let sy = sh as f32 / th as f32;
    let mut out = Vec::with_capacity(tw as usize * th as usize);
    for y in 0..th {
        for x in 0..tw {
            out.push(
                plane.sample_bilinear((x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5),
            );
        }
    }
    out
}

/// Quantizes coverage in [0, 1] to 8-bit (`round(v · 255)`) and compresses
/// it as a raw DEFLATE stream (RFC 1951, no zlib wrapper) — exactly what
/// `focale_core::pipeline::v1` expects to inflate at export.
pub(crate) fn coverage_to_resolved(
    coverage: &[f32],
    w: u32,
    h: u32,
    kind: SegmentKind,
) -> ResolvedMask {
    debug_assert_eq!(coverage.len(), w as usize * h as usize);
    let bytes: Vec<u8> = coverage
        .iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    // Writing to a Vec cannot fail.
    encoder.write_all(&bytes).expect("deflate into Vec");
    let deflate_bitmap = encoder.finish().expect("deflate into Vec");
    ResolvedMask {
        kind,
        width: w,
        height: h,
        deflate_bitmap,
    }
}

/// Resolves a full-resolution coverage plane (`w × h`, values in [0, 1])
/// into a [`ResolvedMask`]: downscale to half resolution (2×2 box filter;
/// odd edges average the in-bounds samples), quantize to 8 bits
/// (`round(v · 255)`), raw-DEFLATE compress.
///
/// # Panics
/// If `coverage.len() != w · h` or either dimension is zero.
pub fn resolve_to_mask(coverage: &[f32], w: u32, h: u32, kind: SegmentKind) -> ResolvedMask {
    assert!(w > 0 && h > 0, "coverage plane must be non-empty");
    assert_eq!(
        coverage.len(),
        w as usize * h as usize,
        "coverage length must be width*height"
    );
    let (hw, hh) = half_dims(w, h);
    let mut half = Vec::with_capacity(hw as usize * hh as usize);
    for y in 0..hh {
        for x in 0..hw {
            let x0 = (2 * x) as usize;
            let y0 = (2 * y) as usize;
            let mut sum = 0.0f32;
            let mut n = 0u32;
            for dy in 0..2usize {
                for dx in 0..2usize {
                    let (sx, sy) = (x0 + dx, y0 + dy);
                    if sx < w as usize && sy < h as usize {
                        sum += coverage[sy * w as usize + sx];
                        n += 1;
                    }
                }
            }
            half.push(sum / n as f32);
        }
    }
    coverage_to_resolved(&half, hw, hh, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    /// Inflates a resolved mask the same way the export rasterizer does.
    fn inflate(mask: &ResolvedMask) -> Vec<u8> {
        let mut bytes = Vec::new();
        DeflateDecoder::new(mask.deflate_bitmap.as_slice())
            .read_to_end(&mut bytes)
            .expect("raw DEFLATE stream");
        assert_eq!(bytes.len(), mask.width as usize * mask.height as usize);
        bytes
    }

    #[test]
    fn resolve_downscales_quantizes_and_roundtrips() {
        // 4×4 plane of 2×2 blocks with known averages.
        #[rustfmt::skip]
        let coverage = [
            1.0, 1.0, 0.0, 0.5,
            1.0, 1.0, 0.5, 0.0,
            0.2, 0.2, 1.0, 1.0,
            0.2, 0.2, 1.0, 1.0,
        ];
        let mask = resolve_to_mask(&coverage, 4, 4, SegmentKind::Subject);
        assert_eq!(mask.kind, SegmentKind::Subject);
        assert_eq!((mask.width, mask.height), (2, 2));
        let bytes = inflate(&mask);
        assert_eq!(bytes, vec![255, 64, 51, 255]); // 1.0, 0.25, 0.2, 1.0
    }

    #[test]
    fn resolve_handles_odd_dimensions() {
        // 3×3: last row/column blocks cover fewer source samples.
        #[rustfmt::skip]
        let coverage = [
            1.0, 1.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
        ];
        let mask = resolve_to_mask(&coverage, 3, 3, SegmentKind::Sky);
        assert_eq!((mask.width, mask.height), (2, 2));
        let bytes = inflate(&mask);
        assert_eq!(bytes, vec![255, 0, 0, 255]);
    }

    #[test]
    fn resolve_single_pixel() {
        let mask = resolve_to_mask(&[0.5], 1, 1, SegmentKind::Object);
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(inflate(&mask), vec![128]); // round(0.5·255)
    }

    #[test]
    fn quantization_clamps_out_of_range_coverage() {
        let mask = coverage_to_resolved(&[-0.5, 1.5], 2, 1, SegmentKind::Subject);
        assert_eq!(inflate(&mask), vec![0, 255]);
    }

    #[test]
    fn sam_scaled_size_longest_side_is_1024() {
        assert_eq!(sam_scaled_size(2048, 1024), (1024, 512));
        assert_eq!(sam_scaled_size(1024, 1024), (1024, 1024));
        assert_eq!(sam_scaled_size(1000, 3000), (341, 1024));
        assert_eq!(sam_scaled_size(1, 4096), (1, 1024)); // min 1 clamp
    }

    #[test]
    fn half_dims_rounds_up_and_clamps() {
        assert_eq!(half_dims(4, 3), (2, 2));
        assert_eq!(half_dims(1, 1), (1, 1));
        assert_eq!(half_dims(5, 8), (3, 4));
    }

    #[test]
    fn srgb_conversion_matches_transfer_curve() {
        // Rec.2020 white maps to sRGB white, black to black.
        assert_eq!(linear_rec2020_to_srgb01([0.0; 3]), [0.0; 3]);
        let white = linear_rec2020_to_srgb01([1.0; 3]);
        for c in white {
            assert!((c - 1.0).abs() < 1e-3, "white → {white:?}");
        }
        // Achromatic mid grey passes through the matrix unchanged (rows sum
        // to 1), so the result is exactly the transfer curve.
        let grey = linear_rec2020_to_srgb01([0.5; 3]);
        for c in grey {
            assert!((c - srgb_encode(0.5)).abs() < 1e-3, "grey → {grey:?}");
        }
    }

    #[test]
    fn resample_identity_at_same_dimensions() {
        let mut img = ImageRgbF32::new(2, 2);
        img.set_pixel(1, 1, [1.0, 1.0, 1.0]);
        let srgb = resample_to_srgb(&img, 2, 2);
        assert_eq!(srgb.len(), 12);
        assert_eq!(srgb[0], 0.0);
        assert!((srgb[9] - 1.0).abs() < 1e-3); // pixel (1,1) stays white
    }

    #[test]
    fn chw_normalization_applies_mean_and_std() {
        // One pixel of sRGB 0.485/0.456/0.406 must normalize to exactly 0.
        let srgb = [
            IMAGENET_MEAN[0],
            IMAGENET_MEAN[1],
            IMAGENET_MEAN[2],
            1.0,
            1.0,
            1.0,
        ];
        let chw = chw_normalized(&srgb, IMAGENET_MEAN, IMAGENET_STD);
        // Planar layout: R plane [px0, px1], then G, then B.
        assert_eq!(chw[0], 0.0);
        assert!((chw[1] - (1.0 - IMAGENET_MEAN[0]) / IMAGENET_STD[0]).abs() < 1e-6);
        assert_eq!(chw[2], 0.0);
        assert_eq!(chw[4], 0.0);
    }

    #[test]
    fn resample_coverage_interpolates() {
        let up = resample_coverage(&[0.0, 1.0], 2, 1, 4, 1);
        assert_eq!(up.len(), 4);
        assert!(up[0] < up[1] && up[1] < up[2] && up[2] < up[3]);
        // Identity when dims match.
        let same = resample_coverage(&[0.25, 0.75], 2, 1, 2, 1);
        assert_eq!(same, vec![0.25, 0.75]);
    }
}
