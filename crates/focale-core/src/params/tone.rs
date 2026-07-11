//! Stage 4: global tone.

use serde::{Deserialize, Serialize};

/// Global tone parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneParams {
    /// Master enable for the whole stage.
    pub enabled: bool,
    /// Exposure compensation in EV stops (−5..=+5).
    pub exposure: f32,
    /// Contrast around middle grey (−100..=+100).
    pub contrast: f32,
    /// Highlight recovery/boost (−100..=+100).
    pub highlights: f32,
    /// Shadow lift/crush (−100..=+100).
    pub shadows: f32,
    /// White point adjustment (−100..=+100).
    pub whites: f32,
    /// Black point adjustment (−100..=+100).
    pub blacks: f32,
    /// Point curve applied after the parametric controls.
    pub curve: ToneCurve,
}

impl Default for ToneParams {
    fn default() -> Self {
        Self {
            enabled: true,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            curve: ToneCurve::default(),
        }
    }
}

/// A point tone curve on the display-referred [0,1] axis, interpolated with
/// a monotone cubic (Fritsch–Carlson). An identity curve is two points at
/// (0,0) and (1,1). Points are kept sorted by `x`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToneCurve {
    /// Luma (RGB-coupled) curve points.
    pub points: Vec<CurvePoint>,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            points: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }],
        }
    }
}

impl ToneCurve {
    /// True when the curve leaves values unchanged.
    pub fn is_identity(&self) -> bool {
        self.points.len() == 2
            && self.points[0] == CurvePoint { x: 0.0, y: 0.0 }
            && self.points[1] == CurvePoint { x: 1.0, y: 1.0 }
    }
}

/// One control point of a point curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// Input position in [0,1].
    pub x: f32,
    /// Output value in [0,1].
    pub y: f32,
}
