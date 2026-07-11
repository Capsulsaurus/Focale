//! JPEG XL export via `jpegxl-rs` (vendored libjxl).
//!
//! Pinned decisions (v1):
//!
//! - **Determinism:** no parallel runner is set, so libjxl runs strictly
//!   single-threaded; effort is pinned to `Squirrel` (7, the libjxl
//!   default) and decoding-speed tier 0.
//! - `distance` drives quality: `0.0` selects true lossless
//!   (`JxlEncoderSetFrameLossless` + `uses_original_profile`, required by
//!   libjxl for bit-exact mode); anything above encodes lossy XYB at that
//!   Butteraugli distance. Valid domain `0.0..=15.0`.
//! - Bit depth 8 or 16 (unsigned integer samples).
//! - **Colour signaling** uses the JPEG XL colour-encoding header (no ICC
//!   payload; the format's native signaling is richer and smaller). Built
//!   through `ColorEncoding::Custom` with an explicit `JxlColorEncoding`:
//!   - SDR sRGB: primaries sRGB, transfer sRGB, D65.
//!   - SDR Display P3: primaries P3 (code 11), transfer sRGB, D65.
//!   - SDR Adobe RGB: **custom primaries** (the published xy values) with
//!     a pure-gamma transfer of exactly 256/563 (encoding exponent).
//!   - SDR Rec.2020: primaries BT.2100 (code 9), transfer sRGB — the v1
//!     Rec.2020-SDR transfer choice (crate docs).
//!   - HDR: primaries BT.2100, transfer PQ (16) or HLG (18), D65, with
//!     `intensity_target` set to the recipe's (clamped) mastering peak.
//!   - Rendering intent: perceptual (matches the ICC profiles the other
//!     formats embed). Samples handed to libjxl are already quantized in
//!     the signaled transfer, so no CMS conversion happens on input.
//! - Output is normally a naked JPEG XL codestream (`FF 0A`); the encoder
//!   is not asked for the ISOBMFF container, but libjxl switches to it on
//!   its own (signature `00 00 00 0C "JXL "`) when the codestream requires
//!   conformance level 10 — observed for 16-bit lossless. Both are valid
//!   JPEG XL files and the choice is deterministic for a given input.

use focale_core::color::adapt::ILLUMINANT_D65;
use focale_core::color::primaries::ADOBE_RGB_PRIMARIES;
use focale_sidecar::schema::{ExportGamut, ExportRecipe, HdrTransfer};
use jpegxl_rs::encode::{ColorEncoding, EncoderSpeed, encoder_builder};
use jpegxl_sys::color::color_encoding::{
    JxlColorEncoding, JxlColorSpace, JxlPrimaries, JxlRenderingIntent, JxlTransferFunction,
    JxlWhitePoint,
};

use crate::ExportError;
use crate::pathway::{SDR_WHITE_NITS, SignalImage};

/// Builds the explicit colour-encoding header (see module docs).
fn color_encoding(recipe: &ExportRecipe) -> JxlColorEncoding {
    let mut enc = JxlColorEncoding {
        color_space: JxlColorSpace::Rgb,
        white_point: JxlWhitePoint::D65,
        white_point_xy: ILLUMINANT_D65,
        primaries: JxlPrimaries::SRgb,
        primaries_red_xy: [0.0; 2],
        primaries_green_xy: [0.0; 2],
        primaries_blue_xy: [0.0; 2],
        transfer_function: JxlTransferFunction::SRGB,
        gamma: 0.0,
        rendering_intent: JxlRenderingIntent::Perceptual,
    };
    match &recipe.hdr {
        Some(hdr) => {
            enc.primaries = JxlPrimaries::Rec2100;
            enc.transfer_function = match hdr.transfer {
                HdrTransfer::Pq => JxlTransferFunction::PQ,
                HdrTransfer::Hlg => JxlTransferFunction::HLG,
            };
        }
        None => match recipe.color.gamut {
            ExportGamut::Srgb => {}
            ExportGamut::DisplayP3 => enc.primaries = JxlPrimaries::P3,
            ExportGamut::Rec2020 => enc.primaries = JxlPrimaries::Rec2100,
            ExportGamut::AdobeRgb => {
                enc.primaries = JxlPrimaries::Custom;
                enc.primaries_red_xy = ADOBE_RGB_PRIMARIES[0];
                enc.primaries_green_xy = ADOBE_RGB_PRIMARIES[1];
                enc.primaries_blue_xy = ADOBE_RGB_PRIMARIES[2];
                enc.transfer_function = JxlTransferFunction::Gamma;
                enc.gamma = 256.0 / 563.0; // encoding exponent of γ 563/256
            }
        },
    }
    enc
}

/// Encodes a JPEG XL codestream (see module docs for the pinned decisions).
pub(crate) fn encode(
    signal: &SignalImage,
    recipe: &ExportRecipe,
    distance: f32,
    bit_depth: u8,
) -> Result<Vec<u8>, ExportError> {
    if !distance.is_finite() || !(0.0..=15.0).contains(&distance) {
        return Err(ExportError::InvalidRecipe(format!(
            "JPEG XL distance must be within 0.0..=15.0, got {distance}"
        )));
    }
    if bit_depth != 8 && bit_depth != 16 {
        return Err(ExportError::InvalidRecipe(format!(
            "JPEG XL bit depth must be 8 or 16, got {bit_depth}"
        )));
    }
    let codec = |e: jpegxl_rs::EncodeError| ExportError::Codec(format!("jpegxl: {e}"));

    let lossless = distance == 0.0;
    let mut encoder = encoder_builder()
        .speed(EncoderSpeed::Squirrel)
        .quality(distance)
        .uses_original_profile(lossless)
        .color_encoding(ColorEncoding::Custom(color_encoding(recipe)))
        .maybe_target_intensity(
            recipe
                .hdr
                .as_ref()
                .map(|hdr| hdr.peak_nits.clamp(SDR_WHITE_NITS, 10_000.0)),
        )
        .build()
        .map_err(codec)?;
    if lossless {
        encoder.lossless = Some(true);
    }

    let data = match bit_depth {
        8 => {
            encoder
                .encode::<u8, u8>(&signal.to_u8(), signal.width, signal.height)
                .map_err(codec)?
                .data
        }
        _ => {
            encoder
                .encode::<u16, u16>(&signal.to_u16(65535), signal.width, signal.height)
                .map_err(codec)?
                .data
        }
    };
    Ok(data)
}
