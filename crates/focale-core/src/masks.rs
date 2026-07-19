//! Mask model (docs/subsystems/masks.md).
//!
//! Masks are stored parametrically in the sidecar and rasterized
//! deterministically on the CPU (`pipeline::v1::masks`). AI-segmented masks
//! are resolved into coverage bitmaps at creation time — a model never runs
//! on the export path.
//!
//! Coordinates are normalized to the pre-geometry working frame: x,y ∈ [0,1]
//! with y down, so masks survive re-crops without re-anchoring.

use serde::{Deserialize, Serialize};

/// A named group of masks combined into one coverage map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskGroup {
    /// User-visible name ("Sky", "Face 1", …).
    pub name: String,
    /// Components combined in order with their [`MaskOp`]s. The first
    /// component's op is applied against an empty (all-zero) coverage.
    pub components: Vec<MaskComponent>,
}

/// One mask layer inside a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskComponent {
    /// How this component combines with the accumulated coverage.
    pub op: MaskOp,
    /// Invert this component's own coverage before combining.
    pub invert: bool,
    /// Edge feather radius as a fraction of the long image edge (0..=0.25).
    pub feather: f32,
    /// Density = maximum opacity scale (0..=1); 1 = full effect.
    pub density: f32,
    /// The mask shape itself.
    pub shape: MaskShape,
}

/// Combination operators (docs/subsystems/masks.md: add / subtract / intersect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskOp {
    /// Union: `max(acc, m)`.
    Add,
    /// Removal: `acc · (1 − m)`.
    Subtract,
    /// Intersection: `acc · m`.
    Intersect,
}

/// All v1 mask shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MaskShape {
    /// Free-hand brush: a list of stamped strokes.
    Brush(BrushMask),
    /// Linear gradient between two parallel lines.
    Linear(LinearGradientMask),
    /// Elliptical radial gradient.
    Radial(RadialGradientMask),
    /// Luminance range selection.
    LuminanceRange(LuminanceRangeMask),
    /// Sampled colour range selection.
    ColorRange(ColorRangeMask),
    /// AI segmentation output, resolved at creation time.
    AiResolved(ResolvedMask),
}

/// A brush mask: ordered strokes, each an ordered list of stamp points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrushMask {
    /// Strokes in paint order (eraser strokes subtract).
    pub strokes: Vec<BrushStroke>,
}

/// One continuous brush drag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrushStroke {
    /// True if this stroke erases instead of paints.
    pub erase: bool,
    /// Brush radius as a fraction of the long image edge.
    pub radius: f32,
    /// Feather fraction of the radius (0 hard .. 1 fully soft).
    pub feather: f32,
    /// Per-stamp opacity accumulation rate (0..=1].
    pub flow: f32,
    /// Polyline of stamp centres (normalized coordinates).
    pub points: Vec<[f32; 2]>,
}

/// Linear gradient: coverage 1 on the `start` side, 0 past `end`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearGradientMask {
    /// Point where coverage is still 1.0 (normalized).
    pub start: [f32; 2],
    /// Point where coverage reaches 0.0 (normalized).
    pub end: [f32; 2],
}

/// Radial gradient: coverage 1 inside the inner ellipse, falling to 0 at the
/// outer boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadialGradientMask {
    /// Ellipse centre (normalized).
    pub center: [f32; 2],
    /// Semi-axes (normalized, x and y).
    pub radius: [f32; 2],
    /// Rotation in degrees CCW.
    pub rotation: f32,
    /// Fraction of the radius over which coverage falls 1→0 (0..=1).
    pub falloff: f32,
}

/// Luminance range mask over working-space luminance Y (Rec.2020 weights).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LuminanceRangeMask {
    /// Lower luminance bound (display-referred [0,1] axis).
    pub low: f32,
    /// Upper luminance bound.
    pub high: f32,
    /// Smoothness of the band edges (0..=1).
    pub falloff: f32,
}

/// Colour range mask: coverage from distance to a sampled colour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorRangeMask {
    /// Sampled reference colour, linear Rec.2020.
    pub color: [f32; 3],
    /// Acceptance radius in Oklab distance (0..=1).
    pub tolerance: f32,
    /// Smoothness of the acceptance edge (0..=1).
    pub falloff: f32,
}

/// An AI mask resolved to a coverage bitmap (docs/subsystems/masks.md: models never run at
/// export). Stored at reduced resolution and upsampled bilinearly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedMask {
    /// What the model segmented, for UI labeling and re-resolve on demand.
    pub kind: SegmentKind,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Deflate-compressed 8-bit coverage, row-major, `width × height` bytes
    /// when decompressed (255 = full coverage).
    #[serde(with = "serde_bytes_vec")]
    pub deflate_bitmap: Vec<u8>,
}

/// What an AI mask selected (docs/subsystems/masks.md parity list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SegmentKind {
    /// Main subject.
    Subject,
    /// Sky.
    Sky,
    /// Everything behind the subject.
    Background,
    /// A clicked/brushed object.
    Object,
    /// A whole person, `index` distinguishing multiple people.
    Person {
        /// Which detected person (0-based).
        index: u8,
    },
    /// A component of a person.
    PersonPart {
        /// Which detected person (0-based).
        index: u8,
        /// The body/face component.
        part: PersonPart,
    },
}

/// Per-person segmentation components (docs/subsystems/masks.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersonPart {
    /// Facial skin.
    FaceSkin,
    /// Non-face skin.
    BodySkin,
    /// Hair.
    Hair,
    /// Eyebrows.
    Eyebrows,
    /// Eye sclera.
    Sclera,
    /// Iris and pupil.
    Iris,
    /// Lips.
    Lips,
    /// Teeth.
    Teeth,
    /// Clothing.
    Clothing,
}

/// Serialize mask bitmaps as CBOR byte strings (major type 2) rather than
/// arrays of integers — an order-of-magnitude size difference in sidecars.
mod serde_bytes_vec {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }
        }
        de.deserialize_byte_buf(V)
    }
}
