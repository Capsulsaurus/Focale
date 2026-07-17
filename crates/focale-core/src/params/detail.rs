//! Stage 7: detail — capture sharpening and conventional noise reduction.
//!
//! Non-neural in v1; neural replacements arrive as new pipeline-versioned
//! stages (architecture.md §3 stage 7).

use serde::{Deserialize, Serialize};

/// Detail-stage parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetailParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Capture sharpening.
    pub sharpen: SharpenParams,
    /// Noise reduction.
    pub noise_reduction: NoiseReductionParams,
}

impl Default for DetailParams {
    fn default() -> Self {
        Self {
            enabled: true,
            sharpen: SharpenParams::default(),
            noise_reduction: NoiseReductionParams::default(),
        }
    }
}

/// Sharpening method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SharpenMethod {
    /// Unsharp mask.
    #[default]
    Unsharp,
    /// Richardson–Lucy deconvolution.
    Deconvolution,
}

/// Capture-sharpening parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharpenParams {
    /// Algorithm selection.
    pub method: SharpenMethod,
    /// Strength (0..=150).
    pub amount: f32,
    /// Kernel radius in pixels (0.5..=3.0).
    pub radius: f32,
    /// Edge masking threshold (0..=100): higher restricts sharpening to
    /// stronger edges.
    pub masking: f32,
}

impl Default for SharpenParams {
    fn default() -> Self {
        Self {
            method: SharpenMethod::Unsharp,
            amount: 40.0,
            radius: 1.0,
            masking: 0.0,
        }
    }
}

/// Conventional (non-neural) noise reduction parameters.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NoiseReductionParams {
    /// Luminance NR strength (0..=100).
    pub luminance: f32,
    /// Luminance detail preservation (0..=100).
    pub luminance_detail: f32,
    /// Chroma NR strength (0..=100).
    pub chroma: f32,
    /// Chroma detail preservation (0..=100).
    pub chroma_detail: f32,
}
