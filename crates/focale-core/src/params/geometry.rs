//! Stage 9: geometry — crop, rotate, perspective.

use serde::{Deserialize, Serialize};

/// Geometry parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeometryParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Crop rectangle in normalized [0,1] coordinates of the (rotated,
    /// perspective-corrected) frame. `None` = full frame.
    pub crop: Option<CropRect>,
    /// Straighten/rotate angle in degrees, counter-clockwise (−45..=45).
    pub rotate: f32,
    /// Perspective correction.
    pub perspective: PerspectiveParams,
    /// Horizontal flip.
    pub flip_horizontal: bool,
}

impl Default for GeometryParams {
    fn default() -> Self {
        Self {
            enabled: true,
            crop: None,
            rotate: 0.0,
            perspective: PerspectiveParams::default(),
            flip_horizontal: false,
        }
    }
}

/// Normalized crop rectangle; 0 ≤ x0 < x1 ≤ 1, 0 ≤ y0 < y1 ≤ 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropRect {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

/// Keystone/perspective correction amounts (−100..=+100 each).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PerspectiveParams {
    /// Vertical keystone (converging verticals).
    pub vertical: f32,
    /// Horizontal keystone.
    pub horizontal: f32,
}
