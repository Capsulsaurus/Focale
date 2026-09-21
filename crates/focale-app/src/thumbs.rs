//! Thumbnails from embedded previews, colour-managed.
//!
//! An embedded preview is *someone else's* rendering, so it arrives tagged
//! with its own colour space and has to be converted before it can share a
//! screen with the viewport. Apple writes Display P3 previews; treating those
//! bytes as sRGB — which is what this module used to do — shows every
//! thumbnail oversaturated, and makes the filmstrip disagree with the image
//! the viewport is painting from the same file.
//!
//! Conversion targets [`viewport::DISPLAY_GAMUT`], the same constant the
//! viewport shader uses, so both surfaces answer to one definition of "what
//! this display is".
//!
//! **Off the deterministic export path** (`[HARD-DET]`). Nothing here feeds
//! an export: `focale-export` generates its own output profiles. This is
//! display-side only, which is why a general CMS is acceptable here and would
//! not be there.

use eframe::egui::ColorImage;
use focale_core::color::Gamut;
use moxcms::{ColorProfile, Layout, TransformOptions};

use crate::viewport;

/// Decodes an embedded JPEG preview into an egui image, downscaled to at most
/// `max_edge` pixels on the long edge.
///
/// Colour is converted from the preview's embedded ICC profile to the display
/// gamut. An untagged preview is assumed sRGB, which is the web/JPEG
/// convention and the only defensible guess. A profile that cannot be parsed
/// or transformed is also treated as sRGB rather than dropping the thumbnail:
/// a slightly wrong colour is more useful than a blank tile, and the file is
/// still browsable.
///
/// `orientation` is an EXIF orientation value (1–8); the preview is rotated
/// to match so portrait frames do not display on their side.
pub fn decode_thumbnail(jpeg: &[u8], max_edge: usize, orientation: u16) -> Option<ColorImage> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    let mut decoder = zune_jpeg::JpegDecoder::new(ZCursor::new(jpeg));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let icc = decoder.icc_profile();
    let pixels = decoder.decode().ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let comps = decoder.output_colorspace()?.num_components();
    if w == 0 || h == 0 || comps < 3 || pixels.len() < w * h * comps {
        return None;
    }

    // Decimate first, convert second: the transform then runs over thumbnail
    // pixels rather than over a 48-megapixel preview.
    let step = (w.max(h)).div_ceil(max_edge).max(1);
    let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgb = Vec::with_capacity(tw * th * 3);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * comps;
            rgb.extend_from_slice(&[pixels[i], pixels[i + 1], pixels[i + 2]]);
        }
    }

    let rgb = to_display_gamut(rgb, icc.as_deref());
    let rgba: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|c| [c[0], c[1], c[2], 255])
        .collect();
    let (rgba, tw, th) = apply_orientation(rgba, tw, th, orientation);
    Some(ColorImage::from_rgba_unmultiplied([tw, th], &rgba))
}

/// Converts interleaved 8-bit RGB from `icc` (or sRGB when absent) into
/// [`viewport::DISPLAY_GAMUT`]. Returns the input unchanged when no
/// conversion is needed or possible.
fn to_display_gamut(rgb: Vec<u8>, icc: Option<&[u8]>) -> Vec<u8> {
    let Some(icc) = icc else {
        // Untagged: assume sRGB. If the display is sRGB there is nothing to
        // do, and if it is not, the assumption is still the right starting
        // point.
        return rgb;
    };
    let Ok(source) = ColorProfile::new_from_slice(icc) else {
        return rgb;
    };
    let destination = match viewport::DISPLAY_GAMUT {
        Gamut::Srgb => ColorProfile::new_srgb(),
        // Only sRGB surfaces exist in v1; when a wider one can be configured
        // (issues #6/#10) this gains the matching profile rather than
        // silently converting to the wrong space.
        _ => ColorProfile::new_srgb(),
    };
    let Ok(transform) = source.create_transform_8bit(
        Layout::Rgb,
        &destination,
        Layout::Rgb,
        TransformOptions::default(),
    ) else {
        return rgb;
    };
    let mut out = vec![0u8; rgb.len()];
    match moxcms::TransformExecutor::transform(&*transform, &rgb, &mut out) {
        Ok(()) => out,
        Err(_) => rgb,
    }
}

/// Rotates/flips 8-bit RGBA pixels per an EXIF orientation value (1–8).
///
/// Returns the possibly-transposed dimensions alongside the pixels. Values
/// outside 1–8, and the identity value 1, are returned untouched.
fn apply_orientation(
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    orientation: u16,
) -> (Vec<u8>, usize, usize) {
    if !(2..=8).contains(&orientation) {
        return (rgba, w, h);
    }
    // Transposing orientations swap the output dimensions.
    let transposed = matches!(orientation, 5..=8);
    let (ow, oh) = if transposed { (h, w) } else { (w, h) };
    let mut out = vec![0u8; rgba.len()];
    for y in 0..h {
        for x in 0..w {
            // Destination coordinate for source (x, y), per EXIF 2.32 §4.6.4.
            let (dx, dy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                8 => (y, w - 1 - x),
                _ => (x, y),
            };
            let src = (y * w + x) * 4;
            let dst = (dy * ow + dx) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (out, ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x1 image: left pixel red, right pixel green.
    fn two_by_one() -> Vec<u8> {
        vec![255, 0, 0, 255, 0, 255, 0, 255]
    }

    #[test]
    fn orientation_1_is_identity() {
        let (out, w, h) = apply_orientation(two_by_one(), 2, 1, 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, two_by_one());
    }

    #[test]
    fn orientation_2_mirrors_horizontally() {
        let (out, w, h) = apply_orientation(two_by_one(), 2, 1, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(&out[0..4], &[0, 255, 0, 255], "green moved to the left");
        assert_eq!(&out[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn orientation_6_transposes_dimensions() {
        let (out, w, h) = apply_orientation(two_by_one(), 2, 1, 6);
        assert_eq!((w, h), (1, 2), "a 2x1 frame rotates to 1x2");
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn every_orientation_covers_all_pixels() {
        // A transform that dropped or doubled a pixel would leave a
        // transparent hole; alpha is 255 everywhere in the source.
        let (w, h) = (3usize, 2usize);
        let src: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        for orientation in 1..=8u16 {
            let (out, ow, oh) = apply_orientation(src.clone(), w, h, orientation);
            assert_eq!(ow * oh, w * h, "orientation {orientation} changed area");
            assert!(
                out.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
                "orientation {orientation} left an unwritten pixel"
            );
        }
    }

    #[test]
    fn out_of_range_orientation_is_identity() {
        for orientation in [0u16, 9, 65535] {
            let (out, w, h) = apply_orientation(two_by_one(), 2, 1, orientation);
            assert_eq!((w, h), (2, 1));
            assert_eq!(out, two_by_one());
        }
    }

    #[test]
    fn untagged_pixels_pass_through_unchanged() {
        let rgb = vec![10, 20, 30, 40, 50, 60];
        assert_eq!(to_display_gamut(rgb.clone(), None), rgb);
    }

    #[test]
    fn unparseable_profile_falls_back_rather_than_dropping_the_image() {
        let rgb = vec![10, 20, 30];
        assert_eq!(
            to_display_gamut(rgb.clone(), Some(b"not an icc profile")),
            rgb
        );
    }

    /// A Display P3 preview must not be shown as if it were sRGB. The same
    /// encoded values mean a more saturated colour in P3, so converting to an
    /// sRGB display has to move them.
    #[test]
    fn display_p3_is_converted_not_passed_through() {
        let icc = ColorProfile::new_display_p3()
            .encode()
            .expect("serialize P3 profile");
        // A mixed, in-gamut colour. Pure primaries are a bad probe: P3's
        // primaries sit outside sRGB, so a relative-colorimetric transform
        // clips them straight back onto sRGB's primaries and looks like a
        // no-op even though it worked.
        let rgb = vec![200u8, 100, 50];
        let out = to_display_gamut(rgb.clone(), Some(&icc));
        assert_ne!(out, rgb, "P3 values were passed through untouched");
        assert!(
            out[0] > rgb[0] && out[2] < rgb[2],
            "converting P3 to the narrower sRGB must increase saturation, got {out:?}"
        );
    }

    /// Both profiles are D65, so the neutral axis must survive the round
    /// trip. A shifted grey is the classic sign of a mismatched white point.
    #[test]
    fn neutral_grey_is_preserved() {
        let icc = ColorProfile::new_display_p3()
            .encode()
            .expect("serialize P3 profile");
        let out = to_display_gamut(vec![128u8, 128, 128], Some(&icc));
        for c in out {
            assert!((i32::from(c) - 128).abs() <= 1, "grey drifted to {c}");
        }
    }
}
