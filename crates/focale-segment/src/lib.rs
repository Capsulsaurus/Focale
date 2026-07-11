//! Local ONNX segmentation for AI masks (PRD §4, docs/architecture.md §6).
//!
//! This crate turns a working-space image into
//! [`focale_core::masks::ResolvedMask`] coverage bitmaps using local ONNX
//! models executed through the `ort` runtime (MIT). Segmentation runs **only
//! at mask-creation time**; the resolved bitmap is what the deterministic
//! export path consumes — a model never runs at export.
//!
//! # Network policy (PRD §2.3: local-only)
//!
//! The application makes **no network calls**. Two downloads exist, both
//! outside the running app:
//!
//! - The `ort` crate's `download-binaries` feature fetches the ONNX Runtime
//!   shared library **at build time only** (a `cargo build` step on the
//!   developer's machine / CI). Nothing is fetched at run time.
//! - Model weights are fetched by the user via `scripts/fetch-models.sh`
//!   (curl + pinned sha256) into the user data directory. When a model file
//!   is absent, this crate reports [`SegmentError::ModelMissing`] and the UI
//!   shows a "model not installed" affordance.
//!
//! # Models
//!
//! All models are redistributable under AGPL-compatible terms and are loaded
//! from `$XDG_DATA_HOME/focale/models` (default `~/.local/share/focale/models`):
//!
//! | [`ModelKind`] | File | Model | License |
//! |---|---|---|---|
//! | `SamEncoder` | `mobile_sam_image_encoder.onnx` | MobileSAM image encoder (ONNX export from `Acly/MobileSAM` on Hugging Face) | Apache-2.0 (upstream MobileSAM); export repo tagged MIT |
//! | `SamDecoder` | `sam_mask_decoder_single.onnx` | SAM mask decoder, single-mask variant (same export repo) | Apache-2.0 / MIT as above |
//! | `FaceParsing` | `face_parsing_resnet18.onnx` | BiSeNet face parsing, ResNet-18 backbone (`yakhyo/face-parsing`, weights retrained from `zllrunning/face-parsing.PyTorch`) | MIT |
//! | `Saliency` | `u2net.onnx` | U²-Net salient-object model (rembg release) | Apache-2.0 |
//! | `Sky` | `skyseg.onnx` | U²-Net sky segmentation (`JianyuanWang/skyseg`, from xiongzhu666/Sky-Segmentation-and-Post-processing) | MIT |
//!
//! # v1 limitations (documented honestly)
//!
//! - **One person.** Person and person-part masks run the face parser on the
//!   full frame and always report person `index` 0. Multi-person masking via
//!   face detection + per-person crops is v2.
//! - **Sclera vs iris.** The 19-class CelebAMask-HQ label set has one eye
//!   class per side, so [`PersonPart::Sclera`] and [`PersonPart::Iris`]
//!   resolve to the same eye region in v1. The UI still offers both (PRD §4
//!   lists "eyes (sclera + iris/pupil)"); a dedicated iris model can split
//!   them in a later pipeline version.
//! - **CPU execution.** Sessions use the default CPU execution provider —
//!   acceptable per PRD §4 ("CPU fallback permitted but may be slow"). GPU
//!   execution providers (CUDA/ROCm/CoreML) are future work; enabling them
//!   is purely a mask-creation-latency improvement and can never affect
//!   export output because only the resolved bitmaps are stored.
//!
//! [`PersonPart::Sclera`]: focale_core::masks::PersonPart::Sclera
//! [`PersonPart::Iris`]: focale_core::masks::PersonPart::Iris

mod error;
mod paths;
mod preprocess;
mod segmenter;

pub use error::SegmentError;
pub use paths::{ModelKind, ModelPaths};
pub use preprocess::{
    FACE_PARSING_SIZE, IMAGENET_MEAN, IMAGENET_STD, SAM_INPUT_SIZE, SAM_PIXEL_MEAN, SAM_PIXEL_STD,
    U2NET_INPUT_SIZE, resolve_to_mask,
};
pub use segmenter::Segmenter;
