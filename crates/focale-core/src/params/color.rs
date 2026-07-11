//! Stage 5: global colour.

use serde::{Deserialize, Serialize};

/// Global colour parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Per-band hue/saturation/luminance adjustments.
    pub hsl: HslBands,
    /// Three-way colour grading wheels.
    pub grading: ColorGrading,
    /// Saturation boost weighted toward muted colours (−100..=+100).
    pub vibrance: f32,
    /// Uniform saturation (−100..=+100).
    pub saturation: f32,
}

impl Default for ColorParams {
    fn default() -> Self {
        Self {
            enabled: true,
            hsl: HslBands::default(),
            grading: ColorGrading::default(),
            vibrance: 0.0,
            saturation: 0.0,
        }
    }
}

/// The eight standard hue bands, in fixed order.
pub const HSL_BAND_COUNT: usize = 8;

/// Names of the hue bands, indexed like the arrays in [`HslBands`].
pub const HSL_BAND_NAMES: [&str; HSL_BAND_COUNT] = [
    "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
];

/// Per-band HSL adjustments. Each entry is −100..=+100, 0 = neutral.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HslBands {
    /// Hue shift per band.
    pub hue: [f32; HSL_BAND_COUNT],
    /// Saturation scale per band.
    pub saturation: [f32; HSL_BAND_COUNT],
    /// Luminance scale per band.
    pub luminance: [f32; HSL_BAND_COUNT],
}

/// Three-way colour grading (shadows / midtones / highlights wheels).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    /// Shadows wheel.
    pub shadows: GradingWheel,
    /// Midtones wheel.
    pub midtones: GradingWheel,
    /// Highlights wheel.
    pub highlights: GradingWheel,
    /// Blending between zones (0..=100).
    pub blending: f32,
    /// Overall balance shift between shadows and highlights (−100..=+100).
    pub balance: f32,
}

/// One colour-grading wheel position.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GradingWheel {
    /// Hue angle in degrees [0, 360).
    pub hue: f32,
    /// Saturation of the tint (0..=100, 0 = no tint).
    pub saturation: f32,
    /// Luminance offset (−100..=+100).
    pub luminance: f32,
}
