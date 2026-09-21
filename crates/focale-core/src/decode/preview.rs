//! Embedded JPEG preview extraction.
//!
//! **Off the deterministic export path.** Previews are what the file's
//! producer baked in, not what Focale's pipeline computes, so nothing here
//! feeds an export ([pipeline](../../../docs/subsystems/pipeline.md)). They
//! exist so a directory can be browsed and culled even when the raw itself
//! cannot be developed — which is most of the time while decode support is
//! still growing (`docs/subsystems/decode.md`).
//!
//! # Why this is not just `RawFile::thumbnail()`
//!
//! rawshift 0.1.1 looks only at `JPEGInterchangeFormat` / `-Length`
//! (0x0201 / 0x0202), the TIFF/EP thumbnail convention. Modern DNG writers
//! — Adobe, and Apple for ProRAW — store previews the other way the DNG
//! specification allows: a full IFD with `NewSubfileType = 1`
//! (reduced-resolution), `Compression = 7` (JPEG), and the bitstream located
//! by `StripOffsets` / `StripByteCounts` (0x0111 / 0x0117).
//!
//! The consequence was that every such file reported "no thumbnail" while
//! carrying a multi-megapixel JPEG that was simply never looked for. This
//! module reads both conventions and returns the largest preview found, so
//! the answer does not depend on which tool wrote the file.

use std::io::{BufReader, Read, Seek};
use std::path::Path;

use rawshift_image::tiff::{CompressionType, Ifd, TiffParser, TiffTag, metadata_helper};

use super::DecodeError;

/// An embedded preview image.
#[derive(Debug, Clone)]
pub struct EmbeddedPreview {
    /// The JPEG bitstream, exactly as stored in the file.
    pub jpeg: Vec<u8>,
    /// Pixel width as declared by the containing IFD, when it declares one.
    ///
    /// Advisory only: it is the container's claim, not the JPEG's own
    /// `SOF` header. Callers that need the true size should read the decoded
    /// image.
    pub width: Option<u32>,
    /// Pixel height as declared by the containing IFD, when it declares one.
    pub height: Option<u32>,
    /// The file's EXIF orientation (1–8) from IFD0, when present.
    ///
    /// Previews are stored in sensor orientation, so a portrait frame arrives
    /// as a landscape JPEG and has to be rotated before display. Returned
    /// here because it comes from the same TIFF parse — asking for it
    /// separately would mean reading the file twice.
    pub orientation: Option<u16>,
}

impl EmbeddedPreview {
    /// Declared pixel count, used to pick the largest candidate. Falls back
    /// to the byte length when the IFD declares no dimensions, which orders
    /// sensibly because a bigger JPEG is almost always a bigger image.
    fn rank(&self) -> u64 {
        match (self.width, self.height) {
            (Some(w), Some(h)) => u64::from(w) * u64::from(h),
            _ => self.jpeg.len() as u64,
        }
    }
}

/// Largest JPEG preview embedded in `path`, or `None` when it carries none.
///
/// Reads both the TIFF/EP thumbnail tags and strip-based preview IFDs (see
/// the module docs). Never decodes raw pixel data, so it stays cheap and
/// works on files whose raw payload Focale cannot decode at all.
pub fn extract_preview(path: &Path) -> Result<Option<EmbeddedPreview>, DecodeError> {
    let file = std::fs::File::open(path)?;
    let mut parser = TiffParser::new(BufReader::new(file))
        .map_err(|e| DecodeError::Decode(format!("TIFF header: {e}")))?;

    let file_size = parser
        .file_size()
        .map_err(|e| DecodeError::Decode(format!("file size: {e}")))?;

    // IFD0 plus the rest of the chain. Errors walking the chain are not
    // fatal: a preview found in IFD0 is still a usable answer.
    let mut ifds = Vec::new();
    match parser.walk_ifd_chain() {
        Ok(chain) => ifds.extend(chain),
        Err(_) => {
            if let Ok(ifd0) = parser.parse_ifd0() {
                ifds.push(ifd0);
            }
        }
    }

    // SubIFDs are where DNG most often keeps its previews.
    let mut all = Vec::new();
    for ifd in &ifds {
        collect(ifd, &mut all);
    }

    // IFD0 carries the orientation that applies to the whole file.
    let orientation = ifds
        .first()
        .and_then(|ifd0| metadata_helper::extract_orientation(&mut parser, ifd0));

    let mut best: Option<EmbeddedPreview> = None;
    for ifd in all {
        for candidate in [
            read_interchange(&mut parser, ifd, file_size),
            read_strips(&mut parser, ifd, file_size),
        ]
        .into_iter()
        .flatten()
        {
            if best.as_ref().is_none_or(|b| candidate.rank() > b.rank()) {
                best = Some(candidate);
            }
        }
    }
    // Stamped once at the end: the orientation belongs to the file, not to
    // whichever IFD the winning preview came from.
    Ok(best.map(|mut p| {
        p.orientation = orientation;
        p
    }))
}

/// Flattens an IFD and its SubIFDs into one list, depth-first.
fn collect<'a>(ifd: &'a Ifd, out: &mut Vec<&'a Ifd>) {
    out.push(ifd);
    for sub in &ifd.sub_ifds {
        collect(sub, out);
    }
}

/// Reads the value of `tag` as a single `u64`.
fn tag_u64<R: Read + Seek>(parser: &mut TiffParser<R>, ifd: &Ifd, tag: TiffTag) -> Option<u64> {
    let entry = ifd.get(tag)?.clone();
    parser.read_value(&entry).ok()?.as_u64()
}

/// Reads the value of `tag` as a list of `u64`.
fn tag_u64_vec<R: Read + Seek>(
    parser: &mut TiffParser<R>,
    ifd: &Ifd,
    tag: TiffTag,
) -> Option<Vec<u64>> {
    let entry = ifd.get(tag)?.clone();
    parser.read_value(&entry).ok()?.as_u64_vec()
}

/// Declared dimensions of the image an IFD describes.
fn dimensions<R: Read + Seek>(parser: &mut TiffParser<R>, ifd: &Ifd) -> (Option<u32>, Option<u32>) {
    let w = tag_u64(parser, ifd, TiffTag::ImageWidth).and_then(|v| u32::try_from(v).ok());
    let h = tag_u64(parser, ifd, TiffTag::ImageLength).and_then(|v| u32::try_from(v).ok());
    (w, h)
}

/// Reads bytes at `offset`, refusing anything that runs past the end of the
/// file. A truncated or hostile file must fail the read, never allocate a
/// buffer sized from an unchecked header field.
fn read_at<R: Read + Seek>(
    parser: &mut TiffParser<R>,
    offset: u64,
    len: u64,
    file_size: u64,
) -> Option<Vec<u8>> {
    if len == 0 || offset.checked_add(len)? > file_size {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    parser.seek_to(offset).ok()?;
    parser.read_bytes(len).ok()
}

/// The TIFF/EP convention: one JPEG located by 0x0201 / 0x0202.
fn read_interchange<R: Read + Seek>(
    parser: &mut TiffParser<R>,
    ifd: &Ifd,
    file_size: u64,
) -> Option<EmbeddedPreview> {
    let offset = tag_u64(parser, ifd, TiffTag::JPEGInterchangeFormat)?;
    let len = tag_u64(parser, ifd, TiffTag::JPEGInterchangeFormatLength)?;
    let jpeg = read_at(parser, offset, len, file_size)?;
    if !is_jpeg(&jpeg) {
        return None;
    }
    // These tags describe the *thumbnail*, while the IFD's ImageWidth /
    // ImageLength describe the main image, so dimensions are not read here.
    Some(EmbeddedPreview {
        jpeg,
        width: None,
        height: None,
        orientation: None,
    })
}

/// The DNG convention: a JPEG-compressed image IFD whose bitstream is
/// located by `StripOffsets` / `StripByteCounts`.
fn read_strips<R: Read + Seek>(
    parser: &mut TiffParser<R>,
    ifd: &Ifd,
    file_size: u64,
) -> Option<EmbeddedPreview> {
    let compression = tag_u64(parser, ifd, TiffTag::Compression)
        .and_then(|v| u16::try_from(v).ok())
        .and_then(CompressionType::from_u16)?;
    // Only baseline JPEG is a preview we can hand to a JPEG decoder. Lossy
    // and JPEG XL DNG payloads are raw image data, not previews.
    if !matches!(
        compression,
        CompressionType::Jpeg | CompressionType::OldJpeg
    ) {
        return None;
    }

    let offsets = tag_u64_vec(parser, ifd, TiffTag::StripOffsets)?;
    let counts = tag_u64_vec(parser, ifd, TiffTag::StripByteCounts)?;
    if offsets.is_empty() || offsets.len() != counts.len() {
        return None;
    }

    // A JPEG-compressed preview is conventionally a single strip. Multi-strip
    // JPEG data is per-strip bitstreams that cannot simply be concatenated
    // into one decodable image, so those are declined rather than corrupted.
    if offsets.len() != 1 {
        return None;
    }

    let jpeg = read_at(parser, offsets[0], counts[0], file_size)?;
    if !is_jpeg(&jpeg) {
        return None;
    }
    let (width, height) = dimensions(parser, ifd);
    Some(EmbeddedPreview {
        jpeg,
        width,
        height,
        orientation: None,
    })
}

/// JPEG SOI marker. Guards against handing a decoder something that merely
/// sat where a JPEG was expected.
fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_jpeg_requires_the_soi_marker() {
        assert!(is_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]));
        assert!(!is_jpeg(&[0xFF, 0xD8]));
        assert!(!is_jpeg(&[0x89, b'P', b'N', b'G']));
        assert!(!is_jpeg(&[]));
    }

    #[test]
    fn rank_prefers_declared_pixels_over_byte_length() {
        let big_bytes = EmbeddedPreview {
            jpeg: vec![0; 10_000],
            width: None,
            height: None,
            orientation: None,
        };
        let small_bytes_many_pixels = EmbeddedPreview {
            jpeg: vec![0; 10],
            width: Some(4032),
            height: Some(3024),
            orientation: None,
        };
        assert!(small_bytes_many_pixels.rank() > big_bytes.rank());
    }

    #[test]
    fn rank_falls_back_to_byte_length_without_dimensions() {
        let a = EmbeddedPreview {
            jpeg: vec![0; 10],
            width: None,
            height: None,
            orientation: None,
        };
        let b = EmbeddedPreview {
            jpeg: vec![0; 20],
            width: None,
            height: None,
            orientation: None,
        };
        assert!(b.rank() > a.rank());
    }

    #[test]
    fn partial_dimensions_fall_back_to_byte_length() {
        let p = EmbeddedPreview {
            jpeg: vec![0; 7],
            width: Some(100),
            height: None,
            orientation: None,
        };
        assert_eq!(p.rank(), 7);
    }

    #[test]
    fn missing_file_is_an_io_error_not_a_panic() {
        let err = extract_preview(Path::new("/nonexistent/focale/none.dng"));
        assert!(matches!(err, Err(DecodeError::Io(_))));
    }
}
