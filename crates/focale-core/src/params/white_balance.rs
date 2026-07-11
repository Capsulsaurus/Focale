//! Stage 3: white balance.

use serde::{Deserialize, Serialize};

/// White balance selection.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum WhiteBalanceParams {
    /// The camera's as-shot multipliers from raw metadata.
    #[default]
    AsShot,
    /// Correlated colour temperature and green–magenta tint.
    Temperature {
        /// Correlated colour temperature in kelvin (typ. 2000–50000).
        kelvin: f32,
        /// Green–magenta tint offset (0 = neutral; negative = green,
        /// positive = magenta; same scale as common raw developers).
        tint: f32,
    },
    /// Raw channel multipliers relative to green (r, g = 1, b), as sampled
    /// from a neutral patch.
    Custom {
        /// Red channel multiplier.
        red: f32,
        /// Blue channel multiplier.
        blue: f32,
    },
}
