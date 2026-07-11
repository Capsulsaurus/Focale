//! Stage 10: finishing — post-crop vignette and grain.

use serde::{Deserialize, Serialize};

/// Finishing parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FinishingParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Post-crop vignette.
    pub vignette: VignetteParams,
    /// Film-style grain.
    pub grain: GrainParams,
}

impl Default for FinishingParams {
    fn default() -> Self {
        Self {
            enabled: true,
            vignette: VignetteParams::default(),
            grain: GrainParams::default(),
        }
    }
}

/// Post-crop vignette parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VignetteParams {
    /// Strength (−100 darken .. +100 lighten; 0 = off).
    pub amount: f32,
    /// Radius at which falloff begins (0..=100).
    pub midpoint: f32,
    /// Shape: −100 rectangular .. +100 circular.
    pub roundness: f32,
    /// Softness of the falloff (0..=100).
    pub feather: f32,
}

impl Default for VignetteParams {
    fn default() -> Self {
        Self {
            amount: 0.0,
            midpoint: 50.0,
            roundness: 0.0,
            feather: 50.0,
        }
    }
}

/// Grain parameters. Grain is procedural, seeded from `seed` so identical
/// sidecars render identical grain (deterministic by construction).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrainParams {
    /// Strength (0..=100; 0 = off).
    pub amount: f32,
    /// Grain particle size (0..=100).
    pub size: f32,
    /// Irregularity of the pattern (0..=100).
    pub roughness: f32,
    /// PRNG seed for the grain field.
    pub seed: u64,
}

impl Default for GrainParams {
    fn default() -> Self {
        Self {
            amount: 0.0,
            size: 25.0,
            roughness: 50.0,
            seed: 0,
        }
    }
}
