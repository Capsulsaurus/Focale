//! Deterministic export encoders for Focale (docs/subsystems/export.md).
//!
//! [`encode`] executes an [`ExportRecipe`] against a rendered working-space
//! image (linear Rec.2020, f32, unbounded — the output of the processing
//! pipeline) and returns the encoded file bytes. Everything in this crate is
//! on the deterministic export path: identical `(image, recipe)` inputs
//! produce bit-identical output bytes.
//!
//! # Determinism rules observed here
//!
//! - All pixel loops are sequential, row-major, fixed order — no `rayon`.
//! - All quantization uses one pinned rounding rule:
//!   `floor(value · maxval + 0.5)`, clamped to `[0, maxval]`.
//! - Every encoder runs single-threaded with pinned settings:
//!   libjxl gets no parallel runner, rav1e is built with
//!   `default-features = false` (no threading) and an explicit
//!   single-thread pool, and `jpeg-encoder` is built without its `simd`
//!   feature (scalar DCT everywhere).
//! - ICC profiles are generated in-crate ([`icc`]) with zeroed
//!   timestamp/profile-ID fields so their bytes never vary.
//! - The `tiff` crate writes native-endian TIFF; every supported target
//!   (x86_64/aarch64 macOS + Linux) is little-endian, so output is
//!   bit-identical across the support matrix (a big-endian port would need
//!   a pipeline-version bump).
//!
//! # Colour pathways (v1 — frozen)
//!
//! - **SDR** (`recipe.hdr == None`): extended-Reinhard tone map
//!   ([`focale_core::color::tonemap_reinhard_extended`], white =
//!   [`focale_core::color::REINHARD_WHITE_DEFAULT`]) → hue-preserving gamut
//!   map into the target gamut ([`focale_core::color::map_to_gamut`]) →
//!   target transfer encode → quantize. Transfer per gamut: sRGB curve for
//!   sRGB and Display P3, the 563/256 gamma for Adobe RGB, and — **v1
//!   pinned choice** — the sRGB curve for a Rec.2020 SDR container (BT.1886
//!   is a display model, not a file encoding; the sRGB curve is what PC
//!   software assumes for SDR RGB payloads).
//! - **HDR** (`recipe.hdr == Some`): no tone mapping. The target gamut is
//!   **forced to Rec.2020** regardless of `recipe.color.gamut` (wide gamut
//!   is the point of HDR output; every HDR container signaling scheme is
//!   built around BT.2020/BT.2100 primaries). Linear 1.0 is anchored at SDR
//!   reference white = 203 cd/m² (ITU-R BT.2408), then PQ or HLG encoded
//!   per [`focale_sidecar::schema::HdrOptions`]. Gain maps are a v2
//!   feature; recipes carrying a `gain_map` block are rejected with
//!   [`ExportError::Unsupported`].
//!
//! Per-format capability and signaling decisions are documented in the
//! format modules (`tiff_out`, `png_out`, `jpeg_out`, `jxl_out`,
//! `avif_out`).

use focale_core::image::ImageRgbF32;
use focale_sidecar::schema::{ExportFormat, ExportRecipe};

pub mod icc;
mod pathway;

mod avif_out;
mod jpeg_out;
mod jxl_out;
mod png_out;
mod tiff_out;

/// Errors produced while executing an export recipe.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The recipe asks for something this version does not support
    /// (e.g. gain maps, HDR TIFF/JPEG, Adobe RGB AVIF).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A recipe field is outside its valid domain
    /// (e.g. a PNG bit depth other than 8/16, JPEG quality 0).
    #[error("invalid export recipe: {0}")]
    InvalidRecipe(String),
    /// The underlying codec reported a failure.
    #[error("codec: {0}")]
    Codec(String),
}

/// Executes an export recipe on a rendered working-space image.
///
/// `image` is linear Rec.2020, unbounded f32 (the output of the pipeline).
/// Returns the encoded file bytes. Deterministic: identical inputs produce
/// bit-identical bytes (see the crate documentation for the pinned rules).
///
/// # Errors
///
/// [`ExportError::Unsupported`] for v1 capability gaps (gain maps, HDR in
/// SDR-only formats, unsignalable gamut/format combinations),
/// [`ExportError::InvalidRecipe`] for out-of-domain recipe values, and
/// [`ExportError::Codec`] if an encoder fails.
pub fn encode(image: &ImageRgbF32, recipe: &ExportRecipe) -> Result<Vec<u8>, ExportError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(ExportError::InvalidRecipe(
            "cannot encode an empty image".into(),
        ));
    }
    if let Some(hdr) = &recipe.hdr
        && hdr.gain_map.is_some()
    {
        return Err(ExportError::Unsupported(
            "gain maps are a v2 feature".into(),
        ));
    }

    let resized = pathway::resize_long_edge(image, recipe.resize);
    let source = resized.as_ref().unwrap_or(image);
    let signal = pathway::to_signal(source, recipe)?;

    match &recipe.format {
        ExportFormat::Tiff16 { compression } => tiff_out::encode(&signal, recipe, *compression),
        ExportFormat::Png { bit_depth } => png_out::encode(&signal, recipe, *bit_depth),
        ExportFormat::Jpeg { quality } => jpeg_out::encode(&signal, recipe, *quality),
        ExportFormat::JpegXl {
            distance,
            bit_depth,
        } => jxl_out::encode(&signal, recipe, *distance, *bit_depth),
        ExportFormat::Avif { quality, bit_depth } => {
            avif_out::encode(&signal, recipe, *quality, *bit_depth)
        }
    }
}
