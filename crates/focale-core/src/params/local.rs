//! Stage 6: local adjustments — masked subsets of the tone (stage 4) and
//! colour (stage 5) parameters.

use serde::{Deserialize, Serialize};

use crate::masks::MaskGroup;
use crate::params::color::GradingWheel;
use crate::params::tone::ToneCurve;

/// One local adjustment: a mask plus the parameter deltas it applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalAdjustment {
    /// Whether this adjustment is active.
    pub enabled: bool,
    /// The mask selecting where the adjustment applies.
    pub mask: MaskGroup,
    /// The adjustment values.
    pub adjustments: LocalParams,
}

/// The subset of global tone/colour parameters available locally (docs/subsystems/pipeline.md stage 6:
/// "any subset of stages 4–5 parameters applied through masks").
///
/// All values are deltas/offsets with 0 = no change, so an empty adjustment
/// is a no-op regardless of the global settings underneath.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalParams {
    /// Exposure offset in EV.
    pub exposure: f32,
    /// Contrast offset (−100..=+100).
    pub contrast: f32,
    /// Highlights offset.
    pub highlights: f32,
    /// Shadows offset.
    pub shadows: f32,
    /// Whites offset.
    pub whites: f32,
    /// Blacks offset.
    pub blacks: f32,
    /// Point curve applied within the mask.
    pub curve: ToneCurve,
    /// White-balance temperature offset (mired-scaled, −100..=+100).
    pub temperature: f32,
    /// Tint offset (−100..=+100).
    pub tint: f32,
    /// A single grading tint applied within the mask.
    pub tint_wheel: GradingWheel,
    /// Vibrance offset.
    pub vibrance: f32,
    /// Saturation offset.
    pub saturation: f32,
}

impl Default for LocalParams {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            curve: ToneCurve::default(),
            temperature: 0.0,
            tint: 0.0,
            tint_wheel: GradingWheel::default(),
            vibrance: 0.0,
            saturation: 0.0,
        }
    }
}
