//! Filmstrip thumbnails from embedded raw previews.

use eframe::egui::ColorImage;

/// Decodes an embedded JPEG preview into an egui image, downscaled to at
/// most `max_edge` pixels on the long edge (nearest-neighbour decimation —
/// thumbnails are not on any colour-critical path).
pub fn decode_thumbnail(jpeg: &[u8], max_edge: usize) -> Option<ColorImage> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    let mut decoder = zune_jpeg::JpegDecoder::new(ZCursor::new(jpeg));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let pixels = decoder.decode().ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let comps = decoder.output_colorspace()?.num_components();
    if comps < 3 || pixels.len() < w * h * comps {
        return None;
    }
    let step = (w.max(h)).div_ceil(max_edge).max(1);
    let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgba = Vec::with_capacity(tw * th * 4);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * comps;
            rgba.extend_from_slice(&[pixels[i], pixels[i + 1], pixels[i + 2], 255]);
        }
    }
    Some(ColorImage::from_rgba_unmultiplied([tw, th], &rgba))
}
