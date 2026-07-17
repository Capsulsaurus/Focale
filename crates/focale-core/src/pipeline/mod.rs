//! The fixed processing pipeline (architecture.md §3).
//!
//! Stage order is permanent and identical for preview and export. Algorithms
//! are grouped by pipeline version: `v1` is frozen at release; changing any
//! algorithm's output requires adding a `v2` module while `v1` remains
//! byte-stable forever (HARD-VER). The dispatcher below is the only place
//! that selects a version.

pub mod v1;

use crate::decode::DecodedRaw;
use crate::image::ImageRgbF32;
use crate::params::EditState;

/// Everything a pipeline run needs.
pub struct RenderInput<'a> {
    /// The decoded raw image (linear camera RGB f32). For preview renders
    /// this may already be downscaled; `scale` says by how much.
    pub decoded: &'a DecodedRaw,
    /// The edit state to apply.
    pub edit: &'a EditState,
    /// Ratio of `decoded`'s resolution to the native raw resolution
    /// (1.0 = full-resolution export). Stages multiply pixel-dimensioned
    /// parameters (blur radii, grain size, …) by this so previews stay
    /// perceptually faithful to the export (HARD-DET).
    pub scale: f32,
}

/// The result of a pipeline run: a working-space image (linear Rec.2020,
/// unbounded) after stages 2–10, ready for an output transform, plus any
/// warnings for the UI/status bar.
pub struct RenderOutput {
    /// The rendered working-space image.
    pub image: ImageRgbF32,
    /// Non-fatal conditions the UI must surface (architecture.md §5, §11).
    pub warnings: Vec<RenderWarning>,
}

/// Non-fatal pipeline conditions surfaced to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderWarning {
    /// The raw file carries no optics-correction metadata; the optics stage
    /// was skipped (architecture.md §5: warn, never guess, never fail).
    OpticsMetadataMissing,
    /// The camera model has no colour calibration; a generic matrix was used.
    CameraMatrixMissing,
    /// The edit was made with an older pipeline version and renders with
    /// that version's algorithms — never silently upgraded. Only an
    /// explicit user action re-stamps a sidecar's `pipeline_version`.
    OlderPipelineVersion(u32),
}

/// Errors from a pipeline run.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The sidecar requests a pipeline version this build does not know.
    #[error("unsupported pipeline version {0} (this build supports 1..={max})", max = crate::PIPELINE_VERSION)]
    UnsupportedVersion(u32),
}

/// Runs the pipeline at `version` (stages 2–10; the output transform is a
/// separate export-side step so previews can reuse the working image).
pub fn render(input: &RenderInput<'_>, version: u32) -> Result<RenderOutput, RenderError> {
    let mut out = match version {
        1 => v1::render(input),
        v => return Err(RenderError::UnsupportedVersion(v)),
    };
    // Unreachable while PIPELINE_VERSION == 1; becomes live the moment a
    // v2 exists, with no dispatcher changes beyond the new match arm.
    if version < crate::PIPELINE_VERSION {
        out.warnings
            .push(RenderWarning::OlderPipelineVersion(version));
    }
    Ok(out)
}
