//! Segmentation error type.

use crate::paths::ModelKind;

/// Errors from model resolution or inference.
#[derive(Debug, thiserror::Error)]
pub enum SegmentError {
    /// The model file is not installed in the models directory. The UI maps
    /// this to a "model not installed" affordance pointing at
    /// `scripts/fetch-models.sh` (the app itself never downloads anything).
    #[error("model not installed: {0:?} (run scripts/fetch-models.sh)")]
    ModelMissing(ModelKind),
    /// The ONNX runtime failed (bad model file, unsupported operator, …).
    #[error("onnx runtime error: {0}")]
    Runtime(String),
    /// The caller passed an unusable input (empty image, point out of range).
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
