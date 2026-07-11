//! Baseline JPEG export: 8-bit, quality-driven, embedded ICC.
//!
//! Pinned decisions (v1):
//!
//! - Chroma subsampling policy: **4:4:4 for quality ≥ 90, 4:2:0 below**.
//!   High-quality exports keep full chroma resolution; web/proof-grade
//!   exports take the standard bandwidth saving.
//! - The ICC profile is embedded via `jpeg-encoder`'s `add_icc_profile`
//!   (standard `ICC_PROFILE` APP2 marker segments).
//! - HDR is rejected: baseline JPEG is an 8-bit SDR container; its HDR
//!   story is gain maps, which are a v2 feature.
//! - `jpeg-encoder` is compiled without its `simd` feature, so the scalar
//!   DCT runs on every architecture — identical bytes everywhere.
//! - JPEG dimensions are 16-bit; images larger than 65535 px on a side
//!   are a codec error.

use focale_sidecar::schema::ExportRecipe;
use jpeg_encoder::{ColorType, Encoder, EncodingError, SamplingFactor};

use crate::pathway::{SignalImage, target_gamut};
use crate::{ExportError, icc};

/// Quality at and above which chroma is kept 4:4:4 (v1 pinned policy).
const FULL_CHROMA_MIN_QUALITY: u8 = 90;

/// Encodes a baseline JPEG (see module docs for the pinned decisions).
pub(crate) fn encode(
    signal: &SignalImage,
    recipe: &ExportRecipe,
    quality: u8,
) -> Result<Vec<u8>, ExportError> {
    if recipe.hdr.is_some() {
        return Err(ExportError::Unsupported(
            "HDR JPEG export is not supported in v1 (gain maps are a v2 feature)".into(),
        ));
    }
    if !(1..=100).contains(&quality) {
        return Err(ExportError::InvalidRecipe(format!(
            "JPEG quality must be 1..=100, got {quality}"
        )));
    }
    if signal.width > 65535 || signal.height > 65535 {
        return Err(ExportError::Codec(format!(
            "JPEG dimensions are limited to 65535, got {}x{}",
            signal.width, signal.height
        )));
    }
    let codec = |e: EncodingError| ExportError::Codec(format!("jpeg: {e}"));

    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(if quality >= FULL_CHROMA_MIN_QUALITY {
        SamplingFactor::F_1_1 // 4:4:4
    } else {
        SamplingFactor::F_2_2 // 4:2:0
    });
    encoder
        .add_icc_profile(&icc::profile(target_gamut(recipe.color.gamut)))
        .map_err(codec)?;
    encoder
        .encode(
            &signal.to_u8(),
            signal.width as u16,
            signal.height as u16,
            ColorType::Rgb,
        )
        .map_err(codec)?;
    Ok(out)
}
