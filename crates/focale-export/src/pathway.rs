//! The shared export colour pathway: resize → tone/gamut map → transfer
//! encode → quantize (v1 — frozen; see the crate docs for the operator
//! definitions).
//!
//! Everything here is sequential, row-major, plain `f32` with fixed
//! expression order — deterministic by construction.

use focale_core::color::{
    Gamut, REINHARD_WHITE_DEFAULT, adobe_rgb_encode, hlg_oetf, map_to_gamut, pq_encode_sdr,
    srgb_encode, tonemap_reinhard_extended,
};
use focale_core::image::ImageRgbF32;
use focale_sidecar::schema::{ExportGamut, ExportRecipe, HdrTransfer, ResizeSpec};

use crate::ExportError;

/// SDR reference white in cd/m² (ITU-R BT.2408): the luminance that
/// working-space linear 1.0 is anchored to on the HDR pathway.
pub const SDR_WHITE_NITS: f32 = 203.0;

/// Maps the sidecar-schema gamut to the colour-math gamut (1:1).
pub(crate) fn target_gamut(gamut: ExportGamut) -> Gamut {
    match gamut {
        ExportGamut::Srgb => Gamut::Srgb,
        ExportGamut::DisplayP3 => Gamut::DisplayP3,
        ExportGamut::AdobeRgb => Gamut::AdobeRgb,
        ExportGamut::Rec2020 => Gamut::Rec2020,
    }
}

/// Resizes to `long_edge` on the longer side, preserving aspect ratio.
///
/// Returns `None` when no resize applies (no spec, `long_edge == 0`, or the
/// image is already at or below the target — never upscales, per the schema
/// contract).
///
/// **v1 pinned resampler:** bilinear, pixel-centre convention
/// (`src = (dst + 0.5) · scale − 0.5`), edge-clamped, via
/// [`ImageRgbF32::sample_bilinear`]. No prefiltering: export resizes are
/// modest downscales where bilinear is adequate; changing the filter
/// changes output bytes and therefore requires a new pipeline version.
/// The short edge is `round(edge · long_edge / long)` (half-up, `f64`),
/// clamped to at least 1 px.
pub(crate) fn resize_long_edge(
    image: &ImageRgbF32,
    spec: Option<ResizeSpec>,
) -> Option<ImageRgbF32> {
    let long_edge = spec?.long_edge;
    let (w, h) = (image.width(), image.height());
    let long = w.max(h);
    if long_edge == 0 || long_edge >= long {
        return None;
    }
    let scale = |edge: u32| -> u32 {
        let exact = f64::from(edge) * f64::from(long_edge) / f64::from(long);
        ((exact + 0.5).floor() as u32).max(1)
    };
    let (new_w, new_h) = if w >= h {
        (long_edge, scale(h))
    } else {
        (scale(w), long_edge)
    };

    let ratio_x = w as f32 / new_w as f32;
    let ratio_y = h as f32 / new_h as f32;
    let mut out = ImageRgbF32::new(new_w, new_h);
    for y in 0..new_h {
        let src_y = (y as f32 + 0.5) * ratio_y - 0.5;
        for x in 0..new_w {
            let src_x = (x as f32 + 0.5) * ratio_x - 0.5;
            out.set_pixel(x, y, image.sample_bilinear(src_x, src_y));
        }
    }
    Some(out)
}

/// A non-linear (transfer-encoded) image ready for quantization:
/// interleaved RGB signal values in `[0, 1]`, row-major.
pub(crate) struct SignalImage {
    pub width: u32,
    pub height: u32,
    pub samples: Vec<f32>,
}

/// Runs the colour pathway (crate docs, "Colour pathways") producing
/// transfer-encoded signal values in `[0, 1]`.
pub(crate) fn to_signal(
    image: &ImageRgbF32,
    recipe: &ExportRecipe,
) -> Result<SignalImage, ExportError> {
    let mut samples = Vec::with_capacity(image.data().len());
    match &recipe.hdr {
        None => {
            let gamut = target_gamut(recipe.color.gamut);
            // Rec.2020 SDR containers use the sRGB transfer (v1 pinned
            // choice, crate docs).
            let encode: fn(f32) -> f32 = match gamut {
                Gamut::Srgb | Gamut::DisplayP3 | Gamut::Rec2020 => srgb_encode,
                Gamut::AdobeRgb => adobe_rgb_encode,
            };
            for px in image.data().chunks_exact(3) {
                let toned =
                    tonemap_reinhard_extended([px[0], px[1], px[2]], REINHARD_WHITE_DEFAULT);
                let mapped = map_to_gamut(toned, gamut);
                samples.extend_from_slice(&[
                    encode(mapped[0]),
                    encode(mapped[1]),
                    encode(mapped[2]),
                ]);
            }
        }
        Some(hdr) => {
            if !hdr.peak_nits.is_finite() || hdr.peak_nits <= 0.0 {
                return Err(ExportError::InvalidRecipe(format!(
                    "HDR peak_nits must be a positive finite number, got {}",
                    hdr.peak_nits
                )));
            }
            // Pinned domain: mastering peak within [SDR white, PQ peak].
            let peak = hdr.peak_nits.clamp(SDR_WHITE_NITS, 10_000.0);
            // HDR output stays in the Rec.2020 working primaries (crate
            // docs); components below 0 clip at 0.
            match hdr.transfer {
                HdrTransfer::Pq => {
                    // Linear 1.0 = 203 cd/m²; values above the mastering
                    // peak clip to it before PQ encoding.
                    let linear_peak = peak / SDR_WHITE_NITS;
                    for c in image.data() {
                        samples.push(pq_encode_sdr(c.clamp(0.0, linear_peak)));
                    }
                }
                HdrTransfer::Hlg => {
                    // HLG is scene-referred with nominal peak at signal
                    // 1.0. v1 pinned mapping: diffuse white (linear 1.0,
                    // 203 cd/m²) sits at 203/peak of the nominal peak, so
                    // scene = linear · 203/peak, clamped to [0, 1] by the
                    // OETF (peak defaults to 1000 cd/m² per the schema).
                    let scale = SDR_WHITE_NITS / peak;
                    for c in image.data() {
                        samples.push(hlg_oetf(c.max(0.0) * scale));
                    }
                }
            }
        }
    }
    Ok(SignalImage {
        width: image.width(),
        height: image.height(),
        samples,
    })
}

/// The pinned export quantizer: `floor(value · maxval + 0.5)`, clamped to
/// `[0, maxval]`. Non-finite input quantizes to 0 (Rust float→int casts
/// saturate and map NaN to 0 — deterministic).
#[inline]
pub(crate) fn quantize_u16(value: f32, maxval: u16) -> u16 {
    let q = (value * f32::from(maxval) + 0.5).floor();
    if q >= f32::from(maxval) {
        maxval
    } else {
        q as u16 // negative and NaN saturate to 0
    }
}

/// [`quantize_u16`] against maxval 255, narrowed to `u8`.
#[inline]
pub(crate) fn quantize_u8(value: f32) -> u8 {
    quantize_u16(value, 255) as u8
}

impl SignalImage {
    /// Quantizes every sample with the pinned rule at 8 bits.
    pub(crate) fn to_u8(&self) -> Vec<u8> {
        self.samples.iter().map(|&v| quantize_u8(v)).collect()
    }

    /// Quantizes every sample with the pinned rule against `maxval`
    /// (65535 for 16-bit containers, 1023/4095 for 10/12-bit AVIF).
    pub(crate) fn to_u16(&self, maxval: u16) -> Vec<u16> {
        self.samples
            .iter()
            .map(|&v| quantize_u16(v, maxval))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizer_pinned_rounding() {
        assert_eq!(quantize_u16(0.0, 65535), 0);
        assert_eq!(quantize_u16(1.0, 65535), 65535);
        assert_eq!(quantize_u16(1.5, 65535), 65535); // clamped
        assert_eq!(quantize_u16(-0.2, 65535), 0); // clamped
        assert_eq!(quantize_u16(f32::NAN, 65535), 0); // NaN → 0
        // 0.5 · 255 = 127.5 → floor(128.0) = 128 (half rounds up).
        assert_eq!(quantize_u8(0.5), 128);
    }

    #[test]
    fn resize_preserves_aspect_and_never_upscales() {
        let img = ImageRgbF32::new(64, 40);
        let out = resize_long_edge(&img, Some(ResizeSpec { long_edge: 32 })).unwrap();
        assert_eq!((out.width(), out.height()), (32, 20));
        assert!(resize_long_edge(&img, Some(ResizeSpec { long_edge: 64 })).is_none());
        assert!(resize_long_edge(&img, Some(ResizeSpec { long_edge: 1000 })).is_none());
        assert!(resize_long_edge(&img, Some(ResizeSpec { long_edge: 0 })).is_none());
        assert!(resize_long_edge(&img, None).is_none());

        let portrait = ImageRgbF32::new(40, 64);
        let out = resize_long_edge(&portrait, Some(ResizeSpec { long_edge: 16 })).unwrap();
        assert_eq!((out.width(), out.height()), (10, 16));
    }

    #[test]
    fn resize_of_constant_image_is_constant() {
        let mut img = ImageRgbF32::new(8, 4);
        for px in img.data_mut().chunks_exact_mut(3) {
            px.copy_from_slice(&[0.25, 0.5, 0.75]);
        }
        let out = resize_long_edge(&img, Some(ResizeSpec { long_edge: 5 })).unwrap();
        for px in out.data().chunks_exact(3) {
            assert_eq!(px, &[0.25, 0.5, 0.75]);
        }
    }
}
