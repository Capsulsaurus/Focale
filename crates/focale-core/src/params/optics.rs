//! Stage 2: optical corrections.
//!
//! v1 applies corrections exclusively from embedded manufacturer metadata in
//! the raw file (architecture.md §5). When metadata is absent the stage is skipped and
//! the UI shows a warning — never guessed, never failed. The per-kind
//! toggles below let the user opt out of individual corrections; they have
//! no effect when the corresponding metadata is missing.

use serde::{Deserialize, Serialize};

/// Optical correction toggles. All default to enabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpticsParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Correct vignetting (falloff) from embedded metadata.
    pub vignetting: bool,
    /// Correct lateral chromatic aberration from embedded metadata.
    pub chromatic_aberration: bool,
    /// Correct geometric distortion from embedded metadata.
    pub distortion: bool,
}

impl Default for OpticsParams {
    fn default() -> Self {
        Self {
            enabled: true,
            vignetting: true,
            chromatic_aberration: true,
            distortion: true,
        }
    }
}
