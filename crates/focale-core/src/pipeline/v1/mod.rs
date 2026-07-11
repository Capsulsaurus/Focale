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

/// Runs pipeline v1, stages 2–10, in the fixed PRD §3 order, producing the
/// working-space image (linear Rec.2020; geometry applied; ready for an
/// output transform).
pub fn render(input: &RenderInput<'_>) -> RenderOutput {
    let mut warnings = Vec::new();
    let meta = &input.decoded.metadata;
    let edit = input.edit;

    // Stage 2: optical corrections. v1 decode exposes no optics metadata
    // (docs/architecture.md §4): warn and skip — never guess, never fail.
    if edit.optics.enabled
        && !(meta.optics.vignetting || meta.optics.chromatic_aberration || meta.optics.distortion)
    {
        warnings.push(RenderWarning::OpticsMetadataMissing);
    }
    if meta.xyz_to_camera.is_none() {
        warnings.push(RenderWarning::CameraMatrixMissing);
    }

    let mut image = ImageRgbF32::from_data(
        input.decoded.width,
        input.decoded.height,
        input.decoded.pixels.clone(),
    );

    // Stage 3: white balance + camera → working space.
    white_balance::apply(&mut image, &edit.white_balance, meta);
    // Stage 4: global tone.
    tone::apply(&mut image, &edit.tone);
    // Stage 5: global colour.
    color_grade::apply(&mut image, &edit.color);
    // Stage 6: local adjustments.
    local::apply(&mut image, &edit.local);
    // Stage 7: detail.
    detail::apply(&mut image, &edit.detail, input.scale);
    // Stage 8: retouch.
    retouch::apply(&mut image, &edit.retouch, input.scale);
    // Stage 9: geometry (orientation + rotation/perspective + crop).
    let image = geometry::apply(&image, &edit.geometry, meta.orientation);
    let mut image = image;
    // Stage 10: finishing.
    finishing::apply(&mut image, &edit.finishing, input.scale);

    RenderOutput { image, warnings }
}
