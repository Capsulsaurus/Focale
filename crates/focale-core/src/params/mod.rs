//! Parameter model for every pipeline stage.
//!
//! These types are the canonical edit state: the sidecar serializes them, the
//! UI edits them, and both the CPU export path and the GPU preview consume
//! them. Stage order is fixed (PRD §3); stages can only be enabled/disabled
//! and parameterized.
//!
//! All fields are plain data with serde derives. Numeric ranges are
//! documented per field and clamped at the UI boundary, not here — the
//! pipeline renders whatever the sidecar says, forever.

pub mod color;
pub mod detail;
pub mod finishing;
pub mod geometry;
pub mod local;
pub mod optics;
pub mod retouch;
pub mod tone;
pub mod white_balance;

pub use color::ColorParams;
pub use detail::DetailParams;
pub use finishing::FinishingParams;
pub use geometry::GeometryParams;
pub use local::LocalAdjustment;
pub use optics::OpticsParams;
pub use retouch::RetouchParams;
pub use tone::ToneParams;
pub use white_balance::WhiteBalanceParams;

use serde::{Deserialize, Serialize};

/// The complete, ordered edit state for one image.
///
/// Field order mirrors the fixed pipeline stage order (PRD §3). Raw decode
/// has no user parameters and the output transform's parameters live in the
/// export recipe, so neither appears here.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditState {
    /// Stage 2: optical corrections from embedded metadata.
    pub optics: OpticsParams,
    /// Stage 3: white balance.
    pub white_balance: WhiteBalanceParams,
    /// Stage 4: global tone.
    pub tone: ToneParams,
    /// Stage 5: global colour.
    pub color: ColorParams,
    /// Stage 6: local adjustments (masked subsets of stages 4–5).
    pub local: Vec<LocalAdjustment>,
    /// Stage 7: detail (capture sharpening + noise reduction).
    pub detail: DetailParams,
    /// Stage 8: retouch (heal / clone strokes).
    pub retouch: RetouchParams,
    /// Stage 9: geometry (crop / rotate / perspective).
    pub geometry: GeometryParams,
    /// Stage 10: finishing (post-crop vignette, grain).
    pub finishing: FinishingParams,
}
