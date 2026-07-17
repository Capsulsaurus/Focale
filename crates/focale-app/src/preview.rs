//! Preview rendering: decode caching, downscaling, pipeline runs.
//!
//! Strategy (docs/architecture.md §8): decode once per image, immediately
//! box-downscale to a preview base (long edge ≤ [`PREVIEW_LONG_EDGE`]) and
//! drop the full-resolution buffer; every slider change re-runs the CPU
//! pipeline on the preview base (stage caching arrives with profiling data;
//! the base is small enough that a full re-run meets the latency budget).
//! The GPU only performs the colour-managed display transform.

use std::path::PathBuf;
use std::sync::Arc;

use focale_core::decode::{DecodeError, DecodedRaw};
use focale_core::image::ImageRgbF32;
use focale_core::params::EditState;
use focale_core::pipeline::{self, RenderInput, RenderWarning};

/// Preview base resolution (long edge, px). ~2560 keeps a full pipeline run
/// well under the 100 ms slider-to-screen budget on the reference machine.
pub const PREVIEW_LONG_EDGE: u32 = 2560;

/// A decoded-and-downscaled image cached for interactive editing.
#[derive(Clone)]
pub struct PreviewBase {
    /// Path this base was decoded from.
    pub path: PathBuf,
    /// Downscaled linear camera RGB (plus metadata) for pipeline input.
    pub decoded: Arc<DecodedRaw>,
    /// Ratio of preview resolution to native (RenderInput::scale).
    pub scale: f32,
}

/// Result of a preview render job.
pub struct PreviewFrame {
    /// Rendered working-space image (linear Rec.2020).
    pub image: ImageRgbF32,
    /// Pipeline warnings to surface in the status bar.
    pub warnings: Vec<RenderWarning>,
    /// Monotonic version for GPU upload deduplication.
    pub version: u64,
}

/// Decodes `path` and builds the preview base.
pub fn build_base(path: &std::path::Path) -> Result<PreviewBase, DecodeError> {
    let full = focale_core::decode::decode_file(path)?;
    let long_edge = full.width.max(full.height);
    let (decoded, scale) = if long_edge <= PREVIEW_LONG_EDGE {
        (full, 1.0)
    } else {
        let factor = long_edge.div_ceil(PREVIEW_LONG_EDGE);
        let scaled = downscale_box(&full, factor);
        let scale = 1.0 / factor as f32;
        (scaled, scale)
    };
    Ok(PreviewBase {
        path: path.to_path_buf(),
        decoded: Arc::new(decoded),
        scale,
    })
}

/// Integer box-filter downscale of the decoded raw by `factor` (average of
/// each factor×factor block; edge blocks average the available pixels).
fn downscale_box(src: &DecodedRaw, factor: u32) -> DecodedRaw {
    let out_w = src.width.div_ceil(factor).max(1);
    let out_h = src.height.div_ceil(factor).max(1);
    let mut pixels = vec![0.0f32; out_w as usize * out_h as usize * 3];
    for oy in 0..out_h {
        let y0 = oy * factor;
        let y1 = (y0 + factor).min(src.height);
        for ox in 0..out_w {
            let x0 = ox * factor;
            let x1 = (x0 + factor).min(src.width);
            let mut acc = [0.0f64; 3];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y as usize * src.width as usize + x as usize) * 3;
                    acc[0] += f64::from(src.pixels[i]);
                    acc[1] += f64::from(src.pixels[i + 1]);
                    acc[2] += f64::from(src.pixels[i + 2]);
                    n += 1;
                }
            }
            let o = (oy as usize * out_w as usize + ox as usize) * 3;
            let inv = 1.0 / f64::from(n);
            pixels[o] = (acc[0] * inv) as f32;
            pixels[o + 1] = (acc[1] * inv) as f32;
            pixels[o + 2] = (acc[2] * inv) as f32;
        }
    }
    DecodedRaw {
        width: out_w,
        height: out_h,
        pixels,
        metadata: src.metadata.clone(),
    }
}

/// Runs the pipeline on the preview base with the sidecar's stored
/// pipeline version — previews must show what an export of that sidecar
/// would produce, never silently the current algorithms. Fails (rather
/// than panics) for versions this build does not implement.
pub fn render(
    base: &PreviewBase,
    edit: &EditState,
    pipeline_version: u32,
    version: u64,
) -> Result<PreviewFrame, pipeline::RenderError> {
    let input = RenderInput {
        decoded: &base.decoded,
        edit,
        scale: base.scale,
    };
    let out = pipeline::render(&input, pipeline_version)?;
    Ok(PreviewFrame {
        image: out.image,
        warnings: out.warnings,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use focale_core::decode::RawMetadata;

    #[test]
    fn downscale_box_averages_blocks() {
        let decoded = DecodedRaw {
            width: 4,
            height: 2,
            pixels: (0..24).map(|i| i as f32).collect(),
            metadata: RawMetadata {
                camera_make: None,
                camera_model: None,
                as_shot_neutral: None,
                xyz_to_camera: None,
                orientation: 1,
                capture_time: None,
                iso: None,
                exposure_time: None,
                f_number: None,
                focal_length: None,
                lens_model: None,
                optics: Default::default(),
            },
        };
        let out = downscale_box(&decoded, 2);
        assert_eq!((out.width, out.height), (2, 1));
        // Block (0,0): pixels 0 and 1 of rows 0/1 → r = mean(0, 3, 12, 15).
        assert_eq!(out.pixels[0], (0.0 + 3.0 + 12.0 + 15.0) / 4.0);
    }

    fn tiny_base() -> PreviewBase {
        PreviewBase {
            path: PathBuf::from("test.dng"),
            decoded: Arc::new(DecodedRaw {
                width: 2,
                height: 2,
                pixels: vec![0.5; 12],
                metadata: RawMetadata {
                    camera_make: None,
                    camera_model: None,
                    as_shot_neutral: None,
                    xyz_to_camera: None,
                    orientation: 1,
                    capture_time: None,
                    iso: None,
                    exposure_time: None,
                    f_number: None,
                    focal_length: None,
                    lens_model: None,
                    optics: Default::default(),
                },
            }),
            scale: 1.0,
        }
    }

    #[test]
    fn render_dispatches_stored_version() {
        let base = tiny_base();
        let edit = EditState::default();
        assert!(render(&base, &edit, focale_core::PIPELINE_VERSION, 0).is_ok());
    }

    #[test]
    fn render_future_version_errors_not_panics() {
        let base = tiny_base();
        let edit = EditState::default();
        assert!(render(&base, &edit, focale_core::PIPELINE_VERSION + 1, 0).is_err());
    }
}
