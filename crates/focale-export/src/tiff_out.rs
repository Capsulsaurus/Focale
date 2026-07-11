//! 16-bit TIFF export — the designated hand-off format (PRD §8).
//!
//! Pinned decisions (v1):
//!
//! - Always RGB, 16 bits per sample, strip layout and defaults as produced
//!   by `tiff` 0.11 (byte-stability is pinned to the locked crate version
//!   like every encoder here).
//! - Compression per [`TiffCompression`]: none, deflate (level 6,
//!   `DeflateLevel::Balanced`) or LZW.
//! - The ICC profile (tag 34675, `InterColorProfile`) is embedded through
//!   the encoder's public `write_tag` API. The `tiff` crate types the byte
//!   payload as BYTE rather than the spec's UNDEFINED; both are 8-bit raw
//!   types and every profile reader tested accepts BYTE (notably libtiff
//!   itself writes BYTE for this tag in some paths). Documented trade-off —
//!   not worth forking the crate over.
//! - HDR is rejected: TIFF has no PQ/HLG signaling convention that
//!   mainstream raster tools honour; the hand-off format stays SDR in v1.
//! - The container is written in native byte order by the `tiff` crate;
//!   all supported targets are little-endian (`II`). See crate docs.

use std::io::Cursor;

use focale_sidecar::schema::{ExportRecipe, TiffCompression};
use tiff::encoder::compression::DeflateLevel;
use tiff::encoder::{Compression, TiffEncoder, colortype};
use tiff::tags::Tag;

use crate::pathway::{SignalImage, target_gamut};
use crate::{ExportError, icc};

/// TIFF `InterColorProfile` tag (34675) holding the raw ICC profile bytes.
const TAG_ICC_PROFILE: u16 = 34675;

/// Encodes a 16-bit RGB TIFF (see module docs for the pinned decisions).
pub(crate) fn encode(
    signal: &SignalImage,
    recipe: &ExportRecipe,
    compression: TiffCompression,
) -> Result<Vec<u8>, ExportError> {
    if recipe.hdr.is_some() {
        return Err(ExportError::Unsupported(
            "HDR TIFF export is not supported in v1 (the hand-off format is SDR)".into(),
        ));
    }
    let codec = |e: tiff::TiffError| ExportError::Codec(format!("tiff: {e}"));

    let compression = match compression {
        TiffCompression::None => Compression::Uncompressed,
        TiffCompression::Deflate => Compression::Deflate(DeflateLevel::Balanced),
        TiffCompression::Lzw => Compression::Lzw,
    };
    let pixels = signal.to_u16(65535);
    let profile = icc::profile(target_gamut(recipe.color.gamut));

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor)
            .map_err(codec)?
            .with_compression(compression);
        let mut image = encoder
            .new_image::<colortype::RGB16>(signal.width, signal.height)
            .map_err(codec)?;
        image
            .encoder()
            .write_tag(Tag::Unknown(TAG_ICC_PROFILE), &profile[..])
            .map_err(codec)?;
        image.write_data(&pixels).map_err(codec)?;
    }
    Ok(cursor.into_inner())
}
