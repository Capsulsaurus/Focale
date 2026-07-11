//! Stage 8: retouch — heal and clone (dust-spot removal).
//!
//! Content-aware inpainting is out of scope for v1 (PRD §3.8).

use serde::{Deserialize, Serialize};

/// Retouch-stage parameters: an ordered list of strokes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetouchParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Strokes applied in order.
    pub strokes: Vec<RetouchStroke>,
}

impl Default for RetouchParams {
    fn default() -> Self {
        Self {
            enabled: true,
            strokes: Vec::new(),
        }
    }
}

/// How a stroke fills its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetouchMode {
    /// Copy source pixels verbatim.
    Clone,
    /// Copy source texture, matching destination luminosity/colour at the
    /// boundary.
    Heal,
}

/// One heal/clone operation: a circular spot or a dragged stroke.
///
/// Coordinates are normalized to the pre-geometry frame (like masks), so
/// retouch survives crop changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetouchStroke {
    /// Heal or clone.
    pub mode: RetouchMode,
    /// Stamp radius as a fraction of the long image edge.
    pub radius: f32,
    /// Edge feather fraction of the radius (0..=1).
    pub feather: f32,
    /// Stroke opacity (0..=1].
    pub opacity: f32,
    /// Destination path (single point = spot removal).
    pub dest: Vec<[f32; 2]>,
    /// Source offset relative to the destination (normalized): source point
    /// = dest point + offset for every stamp.
    pub source_offset: [f32; 2],
}
