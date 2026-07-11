//! PNG export: 8/16-bit RGB, ICC for SDR, cICP for HDR.
//!
//! Pinned decisions (v1):
//!
//! - Bit depth 8 or 16 per the recipe; 16-bit samples are big-endian per
//!   the PNG specification.
//! - Compression `Balanced`, filter `Adaptive` — the `png` 0.18 defaults,
//!   pinned explicitly; both are deterministic (sequential heuristics, no
//!   threading in the `png` crate).
//! - **SDR:** the generated ICC profile ([`crate::icc`]) is embedded as an
//!   `iCCP` chunk via `Info::icc_profile` (the crate zlib-compresses the
//!   profile deterministically).
//! - **HDR:** `png` 0.18 *parses* cICP but its encoder never writes it, so
//!   the cICP chunk is emitted through the public `Writer::write_chunk`
//!   escape hatch immediately after the header chunks (satisfying the
//!   "before PLTE and IDAT" ordering rule; the crate computes the CRC).
//!   Payload: colour primaries 9 (BT.2020), transfer 16 (PQ) or 18 (HLG),
//!   matrix coefficients 0 (RGB), full range 1. No `iCCP` is written for
//!   HDR — PQ/HLG cannot be expressed by a matrix/TRC ICC profile and
//!   cICP takes precedence in PNG third edition anyway.

use focale_sidecar::schema::{ExportRecipe, HdrTransfer};

use crate::pathway::{SignalImage, target_gamut};
use crate::{ExportError, icc};

/// H.273 colour primaries code for BT.2020/BT.2100.
const CICP_PRIMARIES_BT2020: u8 = 9;
/// H.273 transfer characteristics code for SMPTE ST 2084 (PQ).
const CICP_TRANSFER_PQ: u8 = 16;
/// H.273 transfer characteristics code for ARIB STD-B67 (HLG).
const CICP_TRANSFER_HLG: u8 = 18;

/// Encodes a PNG (see module docs for the pinned decisions).
pub(crate) fn encode(
    signal: &SignalImage,
    recipe: &ExportRecipe,
    bit_depth: u8,
) -> Result<Vec<u8>, ExportError> {
    let codec = |e: png::EncodingError| ExportError::Codec(format!("png: {e}"));

    let mut info = png::Info::default();
    info.width = signal.width;
    info.height = signal.height;
    info.color_type = png::ColorType::Rgb;
    info.bit_depth = match bit_depth {
        8 => png::BitDepth::Eight,
        16 => png::BitDepth::Sixteen,
        other => {
            return Err(ExportError::InvalidRecipe(format!(
                "PNG bit depth must be 8 or 16, got {other}"
            )));
        }
    };
    if recipe.hdr.is_none() {
        info.icc_profile = Some(icc::profile(target_gamut(recipe.color.gamut)).into());
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::with_info(&mut out, info).map_err(codec)?;
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder.write_header().map_err(codec)?;

        if let Some(hdr) = &recipe.hdr {
            let transfer = match hdr.transfer {
                HdrTransfer::Pq => CICP_TRANSFER_PQ,
                HdrTransfer::Hlg => CICP_TRANSFER_HLG,
            };
            writer
                .write_chunk(png::chunk::cICP, &[CICP_PRIMARIES_BT2020, transfer, 0, 1])
                .map_err(codec)?;
        }

        let bytes: Vec<u8> = match bit_depth {
            8 => signal.to_u8(),
            _ => signal
                .to_u16(65535)
                .iter()
                .flat_map(|v| v.to_be_bytes())
                .collect(),
        };
        writer.write_image_data(&bytes).map_err(codec)?;
        writer.finish().map_err(codec)?;
    }
    Ok(out)
}
