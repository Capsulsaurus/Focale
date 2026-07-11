//! Mask rasterization for pipeline v1 (PRD §4).
//!
//! Implementation pending. The contract:
//!
//! ```ignore
//! pub struct MaskContext<'a> { /* target dims + working image for range masks */ }
//! pub fn rasterize_group(group: &MaskGroup, ctx: &MaskContext<'_>) -> ImageGrayF32;
//! ```

use crate::image::{ImageGrayF32, ImageRgbF32};
use crate::masks::MaskGroup;

/// Rasterization context: output dimensions plus the working-space image
/// (range masks sample it).
pub struct MaskContext<'a> {
    /// Target mask width (matches the working image).
    pub width: u32,
    /// Target mask height.
    pub height: u32,
    /// The working-space image at the point local adjustments apply
    /// (linear Rec.2020).
    pub image: &'a ImageRgbF32,
}

/// Rasterizes a mask group to a coverage plane in [0,1].
pub fn rasterize_group(_group: &MaskGroup, ctx: &MaskContext<'_>) -> ImageGrayF32 {
    // Placeholder: full coverage. Replaced by the real rasterizer.
    ImageGrayF32::from_data(
        ctx.width,
        ctx.height,
        vec![1.0; ctx.width as usize * ctx.height as usize],
    )
}
