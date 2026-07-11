//! Pipeline version 1 — **frozen at first release**.
//!
//! Every algorithm in this tree is pinned: constants, iteration orders,
//! kernel shapes, interpolation methods. Bug fixes that change output are
//! not allowed here; they become pipeline version 2 (PRD §2.2).
//!
//! Determinism contract (PRD §2.1): CPU-only, f32 with fixed expression
//! order, `rayon` only over disjoint rows, all whole-image statistics
//! computed sequentially, no `HashMap` iteration on the pixel path.

pub mod color_grade;
pub mod detail;
pub mod finishing;
pub mod geometry;
pub mod local;
pub mod masks;
pub mod retouch;
pub mod tone;
pub mod white_balance;

use crate::image::ImageRgbF32;
use crate::pipeline::{RenderInput, RenderOutput, RenderWarning};

/// Runs pipeline v1, stages 2–10, producing the working-space image.
///
/// Stage bodies land module by module; the current skeleton performs the
/// stage-2 warning contract and passes camera RGB through unchanged.
pub fn render(input: &RenderInput<'_>) -> RenderOutput {
    let mut warnings = Vec::new();

    // Stage 2: optical corrections. v1 decode exposes no optics metadata
    // (docs/architecture.md §4): warn and skip, never guess, never fail.
    let meta = &input.decoded.metadata;
    if input.edit.optics.enabled
        && !(meta.optics.vignetting || meta.optics.chromatic_aberration || meta.optics.distortion)
    {
        warnings.push(RenderWarning::OpticsMetadataMissing);
    }
    if meta.xyz_to_camera.is_none() {
        warnings.push(RenderWarning::CameraMatrixMissing);
    }

    let image = ImageRgbF32::from_data(
        input.decoded.width,
        input.decoded.height,
        input.decoded.pixels.clone(),
    );

    RenderOutput { image, warnings }
}
