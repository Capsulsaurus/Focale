//! Lazy ONNX sessions and the per-kind inference paths.

use crate::error::SegmentError;
use crate::paths::{ModelKind, ModelPaths};
use crate::preprocess::{
    FACE_PARSING_SIZE, IMAGENET_MEAN, IMAGENET_STD, U2NET_INPUT_SIZE, chw_normalized,
    coverage_to_resolved, half_dims, resample_coverage, resample_to_srgb, sam_scaled_size,
};
use focale_core::image::ImageRgbF32;
use focale_core::masks::{PersonPart, ResolvedMask, SegmentKind};
use ort::session::Session;
use ort::value::{Tensor, TensorRef};

/// CelebAMask-HQ 19-class label ids, in the `zllrunning/face-parsing.PyTorch`
/// ordering used by the BiSeNet model.
#[allow(dead_code)] // the full label set is documented even where unused
mod face_class {
    pub const BACKGROUND: usize = 0;
    pub const SKIN: usize = 1;
    pub const L_BROW: usize = 2;
    pub const R_BROW: usize = 3;
    pub const L_EYE: usize = 4;
    pub const R_EYE: usize = 5;
    pub const EYE_GLASSES: usize = 6;
    pub const L_EAR: usize = 7;
    pub const R_EAR: usize = 8;
    pub const EAR_RING: usize = 9;
    pub const NOSE: usize = 10;
    pub const MOUTH: usize = 11;
    pub const U_LIP: usize = 12;
    pub const L_LIP: usize = 13;
    pub const NECK: usize = 14;
    pub const NECKLACE: usize = 15;
    pub const CLOTH: usize = 16;
    pub const HAIR: usize = 17;
    pub const HAT: usize = 18;
    /// Number of classes.
    pub const COUNT: usize = 19;
}

/// Which 19-class labels make up each [`PersonPart`].
///
/// | `PersonPart` | classes |
/// |---|---|
/// | `FaceSkin` | skin, nose, l_ear, r_ear (facial skin incl. nose and ears)|
/// | `BodySkin` | neck (the only non-face skin the parser labels) |
/// | `Hair` | hair |
/// | `Eyebrows` | l_brow, r_brow |
/// | `Sclera` | l_eye, r_eye — **v1: same as `Iris`**, see crate docs |
/// | `Iris` | l_eye, r_eye — **v1: same as `Sclera`**, see crate docs |
/// | `Lips` | u_lip, l_lip |
/// | `Teeth` | mouth (the visible mouth interior) |
/// | `Clothing` | cloth, hat |
///
/// Accessory classes (glasses, earring, necklace) belong to no part but are
/// included in the whole-person union.
pub(crate) fn part_classes(part: PersonPart) -> &'static [usize] {
    use face_class as c;
    match part {
        PersonPart::FaceSkin => &[c::SKIN, c::NOSE, c::L_EAR, c::R_EAR],
        PersonPart::BodySkin => &[c::NECK],
        PersonPart::Hair => &[c::HAIR],
        PersonPart::Eyebrows => &[c::L_BROW, c::R_BROW],
        PersonPart::Sclera | PersonPart::Iris => &[c::L_EYE, c::R_EYE],
        PersonPart::Lips => &[c::U_LIP, c::L_LIP],
        PersonPart::Teeth => &[c::MOUTH],
        PersonPart::Clothing => &[c::CLOTH, c::HAT],
    }
}

/// Everything that is a person: all classes except background.
const PERSON_CLASSES: [usize; face_class::COUNT - 1] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
];

/// A cached MobileSAM image embedding (`[1, 256, 64, 64]`).
struct SamEmbedding {
    /// Identity of the image the embedding was computed from.
    key: u64,
    /// Encoder input size (longest side 1024) — point prompts scale to this.
    scaled: (u32, u32),
    /// The raw embedding values.
    data: Vec<f32>,
}

/// Runs local segmentation models and resolves their output into
/// [`ResolvedMask`] bitmaps at half the working resolution.
///
/// Sessions are created lazily on first use of each model and kept for the
/// lifetime of the `Segmenter`. The MobileSAM image embedding is cached (one
/// entry, keyed by an image fingerprint) so repeated [`Self::object_at`]
/// clicks on the same image only re-run the lightweight decoder.
pub struct Segmenter {
    paths: ModelPaths,
    sessions: [Option<Session>; ModelKind::ALL.len()],
    sam_embedding: Option<SamEmbedding>,
}

impl Segmenter {
    /// Creates a segmenter loading models from `paths`. No I/O happens until
    /// the first inference call.
    pub fn new(paths: ModelPaths) -> Self {
        Self {
            paths,
            sessions: [None, None, None, None, None],
            sam_embedding: None,
        }
    }

    /// Reports which models are installed (for the UI's "model not
    /// installed" affordance).
    pub fn available(&self) -> Vec<(ModelKind, bool)> {
        ModelKind::ALL
            .iter()
            .map(|&kind| (kind, self.paths.available(kind)))
            .collect()
    }

    /// Main-subject mask from the U²-Net saliency model.
    ///
    /// `image` is the working-space image (linear Rec.2020); the exact
    /// preprocessing (resize, colour conversion, normalization constants)
    /// is documented in the `preprocess` module and on the re-exported
    /// constants ([`crate::U2NET_INPUT_SIZE`], [`crate::IMAGENET_MEAN`],
    /// [`crate::IMAGENET_STD`]).
    pub fn subject(&mut self, image: &ImageRgbF32) -> Result<ResolvedMask, SegmentError> {
        let coverage = self.u2net_coverage(image, ModelKind::Saliency)?;
        Ok(self.finish_u2net(image, coverage, SegmentKind::Subject))
    }

    /// Background mask: the complement (`1 − coverage`) of [`Self::subject`].
    pub fn background(&mut self, image: &ImageRgbF32) -> Result<ResolvedMask, SegmentError> {
        let mut coverage = self.u2net_coverage(image, ModelKind::Saliency)?;
        for v in &mut coverage {
            *v = 1.0 - *v;
        }
        Ok(self.finish_u2net(image, coverage, SegmentKind::Background))
    }

    /// Sky mask from the U²-Net sky model.
    pub fn sky(&mut self, image: &ImageRgbF32) -> Result<ResolvedMask, SegmentError> {
        let coverage = self.u2net_coverage(image, ModelKind::Sky)?;
        Ok(self.finish_u2net(image, coverage, SegmentKind::Sky))
    }

    /// Click-to-select object mask via MobileSAM.
    ///
    /// `point` is the clicked position in normalized working-frame
    /// coordinates (x, y ∈ [0, 1], y down — the mask coordinate convention
    /// of `focale_core::masks`). The encoder runs once per image (embedding
    /// cached); the decoder runs per click. Decoder output logits pass
    /// through a sigmoid and are kept as **soft** coverage (a logit of 0 —
    /// SAM's binary threshold — maps to coverage 0.5).
    pub fn object_at(
        &mut self,
        image: &ImageRgbF32,
        point: [f32; 2],
    ) -> Result<ResolvedMask, SegmentError> {
        validate_image(image)?;
        if !(0.0..=1.0).contains(&point[0]) || !(0.0..=1.0).contains(&point[1]) {
            return Err(SegmentError::InvalidInput(format!(
                "click point {point:?} outside the normalized [0,1] frame"
            )));
        }
        self.ensure_sam_embedding(image)?;
        let embedding = self.sam_embedding.as_ref().expect("just computed");
        let (sw, sh) = embedding.scaled;
        let (half_w, half_h) = half_dims(image.width(), image.height());

        // Standard SAM ONNX prompt encoding: the click in encoder-input
        // coordinates plus a (0,0) padding point with label −1 (no box).
        let coords = vec![point[0] * sw as f32, point[1] * sh as f32, 0.0, 0.0];
        let labels = vec![1.0f32, -1.0];
        let mask_input = vec![0.0f32; 256 * 256];

        let session = Self::ensure_session(&self.paths, &mut self.sessions, ModelKind::SamDecoder)?;
        let outputs = session
            .run(ort::inputs! {
                "image_embeddings" => TensorRef::from_array_view((
                    [1usize, 256, 64, 64],
                    embedding.data.as_slice(),
                ))
                .map_err(ort_err)?,
                "point_coords" => Tensor::from_array(([1usize, 2, 2], coords)).map_err(ort_err)?,
                "point_labels" => Tensor::from_array(([1usize, 2], labels)).map_err(ort_err)?,
                "mask_input" => Tensor::from_array(([1usize, 1, 256, 256], mask_input))
                    .map_err(ort_err)?,
                "has_mask_input" => Tensor::from_array(([1usize], vec![0.0f32])).map_err(ort_err)?,
                // The decoder resizes its mask to this size — request the
                // half-resolution storage size directly (it preserves the
                // working aspect ratio, which the decoder's internal
                // crop-from-padding math requires).
                "orig_im_size" => Tensor::from_array((
                    [2usize],
                    vec![half_h as f32, half_w as f32],
                ))
                .map_err(ort_err)?,
            })
            .map_err(ort_err)?;
        let masks = outputs
            .get("masks")
            .ok_or_else(|| SegmentError::Runtime("decoder returned no `masks` output".into()))?;
        let (shape, logits) = masks.try_extract_tensor::<f32>().map_err(ort_err)?;
        expect_shape(shape, &[1, 1, half_h as i64, half_w as i64], "SAM decoder")?;
        let coverage: Vec<f32> = logits.iter().map(|&l| sigmoid(l)).collect();
        Ok(coverage_to_resolved(
            &coverage,
            half_w,
            half_h,
            SegmentKind::Object,
        ))
    }

    /// One component of person 0 (v1 parses the full frame as a single
    /// person; see the crate docs). Coverage is the summed softmax
    /// probability of the part's classes — soft edges preserved.
    pub fn person_part(
        &mut self,
        image: &ImageRgbF32,
        part: PersonPart,
    ) -> Result<ResolvedMask, SegmentError> {
        let kind = SegmentKind::PersonPart { index: 0, part };
        self.face_parse_classes(image, part_classes(part), kind)
    }

    /// The whole of person 0: the union (summed probability) of every
    /// non-background parsing class.
    pub fn person(&mut self, image: &ImageRgbF32) -> Result<ResolvedMask, SegmentError> {
        self.face_parse_classes(image, &PERSON_CLASSES, SegmentKind::Person { index: 0 })
    }

    /// Lazily opens the session for `kind` (an associated fn so callers can
    /// keep disjoint borrows of the other `Segmenter` fields).
    fn ensure_session<'s>(
        paths: &ModelPaths,
        sessions: &'s mut [Option<Session>; ModelKind::ALL.len()],
        kind: ModelKind,
    ) -> Result<&'s mut Session, SegmentError> {
        let slot = &mut sessions[kind as usize];
        if slot.is_none() {
            let path = paths.resolve(kind);
            if !path.is_file() {
                return Err(SegmentError::ModelMissing(kind));
            }
            let session = Session::builder()
                .map_err(ort_err)?
                .commit_from_file(&path)
                .map_err(ort_err)?;
            tracing::info!(?kind, path = %path.display(), "loaded segmentation model");
            *slot = Some(session);
        }
        Ok(slot.as_mut().expect("slot filled above"))
    }

    /// Runs a U²-Net-family model and returns its 320×320 saliency map,
    /// min–max normalized to [0, 1] (the rembg post-processing convention;
    /// the network's fused output is already sigmoid-activated).
    fn u2net_coverage(
        &mut self,
        image: &ImageRgbF32,
        kind: ModelKind,
    ) -> Result<Vec<f32>, SegmentError> {
        validate_image(image)?;
        let n = U2NET_INPUT_SIZE;
        let srgb = resample_to_srgb(image, n, n);
        let chw = chw_normalized(&srgb, IMAGENET_MEAN, IMAGENET_STD);
        let session = Self::ensure_session(&self.paths, &mut self.sessions, kind)?;
        let tensor =
            Tensor::from_array(([1usize, 3, n as usize, n as usize], chw)).map_err(ort_err)?;
        let outputs = session.run(ort::inputs![tensor]).map_err(ort_err)?;
        // Output 0 is the fused prediction d0; the other six are the
        // per-stage side outputs.
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(ort_err)?;
        expect_shape(shape, &[1, 1, n as i64, n as i64], "U²-Net")?;
        Ok(min_max_normalize(data))
    }

    /// Resamples a 320×320 U²-Net map to half working resolution and packs
    /// it into a [`ResolvedMask`].
    fn finish_u2net(
        &self,
        image: &ImageRgbF32,
        coverage: Vec<f32>,
        kind: SegmentKind,
    ) -> ResolvedMask {
        let (half_w, half_h) = half_dims(image.width(), image.height());
        let n = U2NET_INPUT_SIZE;
        let resampled = resample_coverage(&coverage, n, n, half_w, half_h);
        coverage_to_resolved(&resampled, half_w, half_h, kind)
    }

    /// Runs the face parser and resolves the summed softmax probability of
    /// `classes` into a half-resolution mask.
    fn face_parse_classes(
        &mut self,
        image: &ImageRgbF32,
        classes: &[usize],
        kind: SegmentKind,
    ) -> Result<ResolvedMask, SegmentError> {
        validate_image(image)?;
        let n = FACE_PARSING_SIZE;
        let srgb = resample_to_srgb(image, n, n);
        let chw = chw_normalized(&srgb, IMAGENET_MEAN, IMAGENET_STD);
        let session =
            Self::ensure_session(&self.paths, &mut self.sessions, ModelKind::FaceParsing)?;
        let tensor =
            Tensor::from_array(([1usize, 3, n as usize, n as usize], chw)).map_err(ort_err)?;
        let outputs = session.run(ort::inputs![tensor]).map_err(ort_err)?;
        // Output 0 is the main head; outputs 1–2 are auxiliary training
        // heads also present in the export.
        let (shape, logits) = outputs[0].try_extract_tensor::<f32>().map_err(ort_err)?;
        expect_shape(
            shape,
            &[1, face_class::COUNT as i64, n as i64, n as i64],
            "face parser",
        )?;
        let coverage = class_probability(logits, classes, n as usize * n as usize);
        let (half_w, half_h) = half_dims(image.width(), image.height());
        let resampled = resample_coverage(&coverage, n, n, half_w, half_h);
        Ok(coverage_to_resolved(&resampled, half_w, half_h, kind))
    }

    /// Computes (or reuses) the MobileSAM embedding for `image`.
    ///
    /// The cache key is a cheap fingerprint — dimensions plus an 8×8 grid of
    /// sampled pixels hashed with FNV-1a — not a cryptographic identity. A
    /// collision would only reuse an embedding for a nearly identical image
    /// at mask-creation time (the user immediately sees the mask), never
    /// affect an export.
    fn ensure_sam_embedding(&mut self, image: &ImageRgbF32) -> Result<(), SegmentError> {
        let key = image_fingerprint(image);
        if self.sam_embedding.as_ref().is_some_and(|e| e.key == key) {
            return Ok(());
        }
        self.sam_embedding = None;
        let (sw, sh) = sam_scaled_size(image.width(), image.height());
        // The Acly encoder export embeds SAM's preprocessing (mean/std
        // normalization and padding to 1024×1024), so it takes HWC RGB in
        // [0, 255] at the aspect-preserving resized size.
        let mut hwc = resample_to_srgb(image, sw, sh);
        for v in &mut hwc {
            *v *= 255.0;
        }
        let session = Self::ensure_session(&self.paths, &mut self.sessions, ModelKind::SamEncoder)?;
        let tensor = Tensor::from_array(([sh as usize, sw as usize, 3], hwc)).map_err(ort_err)?;
        let outputs = session.run(ort::inputs![tensor]).map_err(ort_err)?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(ort_err)?;
        expect_shape(shape, &[1, 256, 64, 64], "SAM encoder")?;
        self.sam_embedding = Some(SamEmbedding {
            key,
            scaled: (sw, sh),
            data: data.to_vec(),
        });
        Ok(())
    }
}

/// Rejects empty images before they reach a model.
fn validate_image(image: &ImageRgbF32) -> Result<(), SegmentError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(SegmentError::InvalidInput("empty image".into()));
    }
    Ok(())
}

/// FNV-1a fingerprint over the dimensions and an 8×8 grid of pixels.
fn image_fingerprint(image: &ImageRgbF32) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    feed(&image.width().to_le_bytes());
    feed(&image.height().to_le_bytes());
    for gy in 0..8u32 {
        for gx in 0..8u32 {
            let x = (gx * image.width().max(1) / 8).min(image.width() - 1);
            let y = (gy * image.height().max(1) / 8).min(image.height() - 1);
            for c in image.pixel(x, y) {
                feed(&c.to_le_bytes());
            }
        }
    }
    hash
}

/// Per-pixel softmax over the 19 class logits (planar `[c][pixel]` layout),
/// summed over `classes`. Fixed iteration order.
fn class_probability(logits: &[f32], classes: &[usize], pixels: usize) -> Vec<f32> {
    let mut coverage = vec![0.0f32; pixels];
    for (i, cov) in coverage.iter_mut().enumerate() {
        let mut max = f32::NEG_INFINITY;
        for c in 0..face_class::COUNT {
            max = max.max(logits[c * pixels + i]);
        }
        let mut denom = 0.0f32;
        for c in 0..face_class::COUNT {
            denom += (logits[c * pixels + i] - max).exp();
        }
        let mut num = 0.0f32;
        for &c in classes {
            num += (logits[c * pixels + i] - max).exp();
        }
        *cov = num / denom;
    }
    coverage
}

/// Min–max normalization to [0, 1]; a flat map just clamps.
fn min_max_normalize(data: &[f32]) -> Vec<f32> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in data {
        min = min.min(v);
        max = max.max(v);
    }
    if max - min > f32::EPSILON {
        data.iter().map(|&v| (v - min) / (max - min)).collect()
    } else {
        data.iter().map(|&v| v.clamp(0.0, 1.0)).collect()
    }
}

/// Logistic sigmoid.
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Validates a runtime tensor shape against the expected one.
fn expect_shape(shape: &[i64], expected: &[i64], what: &str) -> Result<(), SegmentError> {
    if shape != expected {
        return Err(SegmentError::Runtime(format!(
            "{what} returned shape {shape:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

/// Maps an `ort` error into [`SegmentError::Runtime`].
fn ort_err(e: ort::Error) -> SegmentError {
    SegmentError::Runtime(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_dir_segmenter() -> Segmenter {
        Segmenter::new(ModelPaths::new(
            std::env::temp_dir().join("focale-segment-empty-models"),
        ))
    }

    fn test_image() -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(8, 6);
        img.set_pixel(4, 3, [0.5, 0.4, 0.3]);
        img
    }

    #[test]
    fn missing_models_error_per_kind() {
        let mut seg = empty_dir_segmenter();
        let img = test_image();
        assert!(matches!(
            seg.subject(&img),
            Err(SegmentError::ModelMissing(ModelKind::Saliency))
        ));
        assert!(matches!(
            seg.sky(&img),
            Err(SegmentError::ModelMissing(ModelKind::Sky))
        ));
        assert!(matches!(
            seg.object_at(&img, [0.5, 0.5]),
            Err(SegmentError::ModelMissing(ModelKind::SamEncoder))
        ));
        assert!(matches!(
            seg.person(&img),
            Err(SegmentError::ModelMissing(ModelKind::FaceParsing))
        ));
        assert!(matches!(
            seg.person_part(&img, PersonPart::Hair),
            Err(SegmentError::ModelMissing(ModelKind::FaceParsing))
        ));
    }

    #[test]
    fn available_reports_all_kinds_missing() {
        let seg = empty_dir_segmenter();
        let avail = seg.available();
        assert_eq!(avail.len(), ModelKind::ALL.len());
        assert!(avail.iter().all(|(_, present)| !present));
    }

    #[test]
    fn empty_image_is_invalid_input() {
        let mut seg = empty_dir_segmenter();
        let img = ImageRgbF32::new(0, 0);
        assert!(matches!(
            seg.subject(&img),
            Err(SegmentError::InvalidInput(_))
        ));
    }

    #[test]
    fn out_of_range_click_is_invalid_input() {
        let mut seg = empty_dir_segmenter();
        let img = test_image();
        assert!(matches!(
            seg.object_at(&img, [1.5, 0.5]),
            Err(SegmentError::InvalidInput(_))
        ));
        assert!(matches!(
            seg.object_at(&img, [0.5, -0.1]),
            Err(SegmentError::InvalidInput(_))
        ));
    }

    #[test]
    fn part_class_mapping_matches_documented_table() {
        use face_class as c;
        assert_eq!(
            part_classes(PersonPart::FaceSkin),
            &[c::SKIN, c::NOSE, c::L_EAR, c::R_EAR]
        );
        assert_eq!(part_classes(PersonPart::BodySkin), &[c::NECK]);
        assert_eq!(part_classes(PersonPart::Hair), &[c::HAIR]);
        assert_eq!(part_classes(PersonPart::Eyebrows), &[c::L_BROW, c::R_BROW]);
        // v1: sclera and iris both resolve to the eye classes.
        assert_eq!(part_classes(PersonPart::Sclera), &[c::L_EYE, c::R_EYE]);
        assert_eq!(
            part_classes(PersonPart::Sclera),
            part_classes(PersonPart::Iris)
        );
        assert_eq!(part_classes(PersonPart::Lips), &[c::U_LIP, c::L_LIP]);
        assert_eq!(part_classes(PersonPart::Teeth), &[c::MOUTH]);
        assert_eq!(part_classes(PersonPart::Clothing), &[c::CLOTH, c::HAT]);
        // The person union covers every non-background class.
        assert_eq!(PERSON_CLASSES.len(), c::COUNT - 1);
        assert!(!PERSON_CLASSES.contains(&c::BACKGROUND));
    }

    #[test]
    fn fingerprint_tracks_content_and_dimensions() {
        let a = test_image();
        let b = test_image();
        assert_eq!(image_fingerprint(&a), image_fingerprint(&b));
        let mut c = test_image();
        c.set_pixel(0, 0, [0.1, 0.0, 0.0]);
        assert_ne!(image_fingerprint(&a), image_fingerprint(&c));
        let d = ImageRgbF32::new(6, 8);
        assert_ne!(
            image_fingerprint(&ImageRgbF32::new(8, 6)),
            image_fingerprint(&d)
        );
    }

    #[test]
    fn class_probability_sums_softmax_terms() {
        // 2 pixels, 19 classes: pixel 0 strongly class 1, pixel 1 strongly
        // class 0 (background).
        let pixels = 2;
        let mut logits = vec![0.0f32; face_class::COUNT * pixels];
        logits[face_class::SKIN * pixels] = 20.0; // pixel 0, class skin
        logits[face_class::BACKGROUND * pixels + 1] = 20.0; // pixel 1, background
        let cov = class_probability(&logits, &[face_class::SKIN], pixels);
        assert!(cov[0] > 0.99, "skin pixel: {}", cov[0]);
        assert!(cov[1] < 0.01, "background pixel: {}", cov[1]);
        // Summing all classes gives probability 1 everywhere.
        let all: Vec<usize> = (0..face_class::COUNT).collect();
        let cov = class_probability(&logits, &all, pixels);
        assert!(cov.iter().all(|&v| (v - 1.0).abs() < 1e-5));
    }

    #[test]
    fn min_max_normalize_spans_unit_range() {
        let n = min_max_normalize(&[2.0, 4.0, 3.0]);
        assert_eq!(n, vec![0.0, 1.0, 0.5]);
        // Flat input clamps instead of dividing by zero.
        let flat = min_max_normalize(&[7.0, 7.0]);
        assert_eq!(flat, vec![1.0, 1.0]);
    }
}
