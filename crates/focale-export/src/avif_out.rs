//! AVIF export: rav1e still-picture AV1 payload wrapped by avif-serialize.
//!
//! Pinned decisions (v1):
//!
//! - **Determinism:** rav1e is built with `default-features = false` (no
//!   rayon threading, no asm) and additionally pinned to a single-worker
//!   thread pool (`with_threads(1)`), speed preset **6**, still-picture
//!   mode, low-latency (no frame reordering), single tile. With a fixed
//!   configuration rav1e's RDO search is a pure function of its input —
//!   verified by the double-encode byte-compare test.
//! - **Pixel layout:** 4:4:4 (no chroma subsampling), full range, bit
//!   depth 8, 10 or 12 per the recipe.
//! - **Matrix (v1 pinned):** SDR uses matrix coefficients 1 (BT.709,
//!   Kr = 0.2126 / Kb = 0.0722); HDR uses 9 (BT.2020 non-constant
//!   luminance, Kr = 0.2627 / Kb = 0.0593). The RGB→YCbCr conversion runs
//!   on the non-linear signal in `f32` with the pinned quantizer; chroma
//!   is offset by `2^(depth−1)` per H.273 full-range.
//! - **CICP:** SDR sRGB → primaries 1 + transfer 13; Display P3 →
//!   primaries 12 + transfer 13; Rec.2020 SDR → primaries 9 + transfer 13
//!   (the v1 Rec.2020-SDR transfer choice); HDR → primaries 9 + transfer
//!   16 (PQ) or 18 (HLG). The same triple is signaled both in the AV1
//!   sequence header (rav1e `color_description`) and the container `colr`
//!   box (avif-serialize). **Adobe RGB is rejected**: H.273 defines no
//!   code point for its primaries and avif-serialize embeds no ICC, so an
//!   Adobe RGB AVIF could not be labelled truthfully.
//! - **Quality → quantizer:** `quantizer = (100 − quality) · 255 / 99`
//!   (integer maths), i.e. quality 100 → 0, quality 1 → 255.
//! - AV1 sequence profile 1 (4:4:4, 8/10-bit) or 2 (12-bit).

use avif_serialize::Aviffy;
use avif_serialize::constants as avifc;
use focale_sidecar::schema::{ExportGamut, ExportRecipe, HdrTransfer};
use rav1e::prelude::*;

use crate::ExportError;
use crate::pathway::SignalImage;

/// BT.709 luma coefficients (Kr, Kb) — SDR matrix (H.273 code 1).
const KR_KB_BT709: (f32, f32) = (0.2126, 0.0722);
/// BT.2020 NCL luma coefficients (Kr, Kb) — HDR matrix (H.273 code 9).
const KR_KB_BT2020: (f32, f32) = (0.2627, 0.0593);

/// The CICP triple for a recipe, held as both the rav1e (sequence header)
/// and avif-serialize (`colr` box) enums, plus the matching luma
/// coefficients (see module docs).
struct Cicp {
    rav1e: ColorDescription,
    avif_primaries: avifc::ColorPrimaries,
    avif_transfer: avifc::TransferCharacteristics,
    avif_matrix: avifc::MatrixCoefficients,
    kr_kb: (f32, f32),
}

fn cicp(recipe: &ExportRecipe) -> Result<Cicp, ExportError> {
    match &recipe.hdr {
        Some(hdr) => {
            let (transfer, avif_transfer) = match hdr.transfer {
                HdrTransfer::Pq => (
                    TransferCharacteristics::SMPTE2084,
                    avifc::TransferCharacteristics::Smpte2084,
                ),
                HdrTransfer::Hlg => (
                    TransferCharacteristics::HLG,
                    avifc::TransferCharacteristics::Hlg,
                ),
            };
            Ok(Cicp {
                rav1e: ColorDescription {
                    color_primaries: ColorPrimaries::BT2020,
                    transfer_characteristics: transfer,
                    matrix_coefficients: MatrixCoefficients::BT2020NCL,
                },
                avif_primaries: avifc::ColorPrimaries::Bt2020,
                avif_transfer,
                avif_matrix: avifc::MatrixCoefficients::Bt2020Ncl,
                kr_kb: KR_KB_BT2020,
            })
        }
        None => {
            let (primaries, avif_primaries) = match recipe.color.gamut {
                ExportGamut::Srgb => (ColorPrimaries::BT709, avifc::ColorPrimaries::Bt709),
                ExportGamut::DisplayP3 => {
                    (ColorPrimaries::SMPTE432, avifc::ColorPrimaries::DisplayP3)
                }
                ExportGamut::Rec2020 => (ColorPrimaries::BT2020, avifc::ColorPrimaries::Bt2020),
                ExportGamut::AdobeRgb => {
                    return Err(ExportError::Unsupported(
                        "AVIF cannot signal Adobe RGB (1998) primaries (no H.273 code point); \
                         choose sRGB, Display P3 or Rec. 2020"
                            .into(),
                    ));
                }
            };
            Ok(Cicp {
                rav1e: ColorDescription {
                    color_primaries: primaries,
                    transfer_characteristics: TransferCharacteristics::SRGB,
                    matrix_coefficients: MatrixCoefficients::BT709,
                },
                avif_primaries,
                avif_transfer: avifc::TransferCharacteristics::Srgb,
                avif_matrix: avifc::MatrixCoefficients::Bt709,
                kr_kb: KR_KB_BT709,
            })
        }
    }
}

/// Converts the signal image to full-range 4:4:4 YCbCr planes, quantized
/// to `bit_depth` and serialized as little-endian bytes (1 byte/sample for
/// depth 8, 2 for 10/12) for `Plane::copy_from_raw_u8`.
fn ycbcr_planes(signal: &SignalImage, bit_depth: u8, kr_kb: (f32, f32)) -> [Vec<u8>; 3] {
    let (kr, kb) = kr_kb;
    let kg = 1.0 - kr - kb;
    let maxval = ((1u32 << bit_depth) - 1) as f32;
    let offset = (1u32 << (bit_depth - 1)) as f32;
    let quant = |v: f32| -> u16 {
        let q = (v + 0.5).floor();
        if q >= maxval {
            maxval as u16
        } else {
            q as u16 // negative and NaN saturate to 0
        }
    };
    let n = signal.samples.len() / 3;
    let mut planes = [
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    ];
    for px in signal.samples.chunks_exact(3) {
        let (r, g, b) = (px[0], px[1], px[2]);
        let y = kr * r + kg * g + kb * b;
        let cb = (b - y) / (2.0 * (1.0 - kb));
        let cr = (r - y) / (2.0 * (1.0 - kr));
        planes[0].push(quant(y * maxval));
        planes[1].push(quant(cb * maxval + offset));
        planes[2].push(quant(cr * maxval + offset));
    }
    planes.map(|plane| {
        if bit_depth == 8 {
            plane.iter().map(|&v| v as u8).collect()
        } else {
            plane.iter().flat_map(|v| v.to_le_bytes()).collect()
        }
    })
}

/// Runs rav1e over the planes with the pinned configuration, returning the
/// still-picture AV1 payload.
fn encode_av1<T: Pixel>(
    planes: &[Vec<u8>; 3],
    signal: &SignalImage,
    bit_depth: u8,
    quantizer: usize,
    color: &Cicp,
) -> Result<Vec<u8>, ExportError> {
    let codec = |m: String| ExportError::Codec(format!("rav1e: {m}"));

    let mut enc = EncoderConfig::with_speed_preset(6);
    enc.width = signal.width as usize;
    enc.height = signal.height as usize;
    enc.bit_depth = bit_depth as usize;
    enc.chroma_sampling = ChromaSampling::Cs444;
    enc.pixel_range = PixelRange::Full;
    enc.color_description = Some(color.rav1e);
    enc.still_picture = true;
    enc.low_latency = true;
    enc.min_key_frame_interval = 0;
    enc.max_key_frame_interval = 1;
    enc.quantizer = quantizer;
    enc.tiles = 1;

    let config = Config::new().with_encoder_config(enc).with_threads(1);
    let mut ctx: Context<T> = config
        .new_context()
        .map_err(|e| codec(format!("invalid config: {e}")))?;

    let mut frame = ctx.new_frame();
    let bytewidth = if bit_depth == 8 { 1 } else { 2 };
    for (plane, bytes) in frame.planes.iter_mut().zip(planes) {
        plane.copy_from_raw_u8(bytes, signal.width as usize * bytewidth, bytewidth);
    }
    ctx.send_frame(frame)
        .map_err(|e| codec(format!("send_frame: {e}")))?;
    ctx.flush();

    let mut payload: Option<Vec<u8>> = None;
    loop {
        match ctx.receive_packet() {
            Ok(packet) => {
                if payload.replace(packet.data).is_some() {
                    return Err(codec("expected exactly one still-picture packet".into()));
                }
            }
            Err(EncoderStatus::Encoded) => {}
            Err(EncoderStatus::LimitReached) => break,
            Err(e) => return Err(codec(format!("receive_packet: {e}"))),
        }
    }
    payload.ok_or_else(|| codec("encoder produced no packet".into()))
}

/// Encodes an AVIF file (see module docs for the pinned decisions).
pub(crate) fn encode(
    signal: &SignalImage,
    recipe: &ExportRecipe,
    quality: u8,
    bit_depth: u8,
) -> Result<Vec<u8>, ExportError> {
    if !(1..=100).contains(&quality) {
        return Err(ExportError::InvalidRecipe(format!(
            "AVIF quality must be 1..=100, got {quality}"
        )));
    }
    if ![8, 10, 12].contains(&bit_depth) {
        return Err(ExportError::InvalidRecipe(format!(
            "AVIF bit depth must be 8, 10 or 12, got {bit_depth}"
        )));
    }
    let color = cicp(recipe)?;
    let planes = ycbcr_planes(signal, bit_depth, color.kr_kb);
    let quantizer = (100 - quality as usize) * 255 / 99;

    let av1 = if bit_depth == 8 {
        encode_av1::<u8>(&planes, signal, bit_depth, quantizer, &color)?
    } else {
        encode_av1::<u16>(&planes, signal, bit_depth, quantizer, &color)?
    };

    let mut aviffy = Aviffy::new();
    aviffy
        .set_color_primaries(color.avif_primaries)
        .set_transfer_characteristics(color.avif_transfer)
        .set_matrix_coefficients(color.avif_matrix)
        .set_full_color_range(true)
        .set_chroma_subsampling((false, false))
        .set_seq_profile(if bit_depth == 12 { 2 } else { 1 });
    Ok(aviffy.to_vec(&av1, None, signal.width, signal.height, bit_depth))
}
