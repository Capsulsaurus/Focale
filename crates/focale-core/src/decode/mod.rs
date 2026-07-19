//! Stage 1: raw decode — rawshift-backed ARW/DNG decoding to linear camera
//! RGB f32 (docs/subsystems/decode.md, docs/subsystems/pipeline.md stage 1).
//!
//! # Pixel pipeline
//!
//! [`decode_file`] runs, in fixed order:
//!
//! 1. `RawFile::open` — format auto-detection (Sony ARW, Adobe DNG).
//! 2. `decode_raw` — u16 Bayer CFA data at native bit depth.
//! 3. `apply_black_level` — per-CFA-site pedestal subtraction (saturating).
//! 4. Demosaic — **pinned to `Bayer(Amaze)`**, never `Auto`: `Auto` is
//!    content-adaptive and would violate the permanent-versioning rule.
//!    Output covers the sensor's *active area* only, so [`DecodedRaw::width`]
//!    and [`DecodedRaw::height`] are the active-area dimensions.
//! 5. Normalization to f32. Exact formula, each division individually
//!    rounded (no reciprocal-multiply), in a single sequential loop:
//!
//!    ```text
//!    pixel_f32 = demosaiced_u16 / (white_level − max(black_levels))
//!    ```
//!
//!    The demosaic input is pedestal-subtracted, so the effective saturation
//!    point is `white_level − black_level`. Black levels are per CFA site;
//!    the *maximum* of the four is used as the single divisor so that a
//!    saturated sensor value always maps to ≥ 1.0 (channels with a smaller
//!    pedestal may slightly exceed 1.0 — the working space is unbounded and
//!    downstream stages must not assume [0, 1]). In practice the four levels
//!    are equal on every supported body, making this exactly the DNG
//!    `(v − BlackLevel) / (WhiteLevel − BlackLevel)` mapping. rawshift's own
//!    `process()` uses the same `white_level − black_level` effective white
//!    but rescales to u16 before demosaicing; we divide once, after
//!    demosaicing, in f32, which avoids the intermediate quantization.
//!
//! Orientation is **not** baked into the pixels; it is reported in
//! [`RawMetadata::orientation`] and handled by the geometry stage / viewport.
//!
//! # Determinism
//!
//! Everything here is CPU-only and bit-identical across machines: the
//! normalization loop is sequential in row-major order, no `HashMap`
//! iteration touches pixels, and rawshift's AMaZE demosaic parallelizes over
//! *disjoint output rows* computed from immutable input (`par_chunks_mut`
//! per row), so its output is independent of thread count and scheduling.
//!
//! # Format support and typed errors
//!
//! Supported today: Sony lossless-compressed ARW (compression type 7) and
//! Bayer-CFA DNG. Uncompressed / lossy-compressed ARW surfaces as
//! [`DecodeError::UnsupportedCompression`]; pre-demosaiced "LinearRaw" DNG
//! (e.g. Apple ProRAW) as [`DecodeError::UnsupportedFormat`].
//!
//! # Optics correction metadata (docs/subsystems/optics.md)
//!
//! The optics stage (docs/subsystems/optics.md) sources corrections exclusively from embedded
//! metadata. rawshift 0.1.1 parses **no** optics metadata from ARW (Sony
//! stores it in undecoded MakerNote tags) and applies DNG `GainMap` opcodes
//! only on its internal LinearRaw path, which we do not use. Consequently
//! [`OpticsMetadata`] reports all sources absent (`false`) in v1, and the
//! optics stage emits its mandated (docs/subsystems/optics.md) "no optics metadata; stage skipped"
//! warning. The struct exists so presence is reported honestly per file the
//! moment upstream parsing lands.
//!
//! # Colour matrix
//!
//! [`RawMetadata::xyz_to_camera`] is the DNG-convention XYZ→camera matrix.
//! DNG files use their embedded `ColorMatrix1/2`; ARW falls back to
//! rawshift's bundled per-model calibration database. When two calibration
//! matrices are available (dual-illuminant), they are blended at the scene's
//! correlated colour temperature — estimated from the as-shot neutral —
//! linearly in reciprocal CCT (mired), the standard DNG practice, via
//! rawshift's `interpolate_color_matrix`. With a single matrix, it is used
//! as-is (`ColorMatrix2`, conventionally the D65 calibration, is preferred
//! when interpolation inputs are missing).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use rawshift_image::core::ImageMetadata;
use rawshift_image::core::metadata::URational;
use rawshift_image::data::cameras::find_camera_calibration;
use rawshift_image::error::RawError;
use rawshift_image::formats::RawFile;
use rawshift_image::processing::{BayerAlgorithm, DemosaicMethod};
use rawshift_image::transforms::apply_black_level;
use rawshift_image::transforms::color::{
    estimate_cct_from_as_shot_neutral, interpolate_color_matrix,
};
use thiserror::Error;

/// A decoded raw file: linear camera-native RGB pixels plus the metadata the
/// rest of the pipeline needs.
#[derive(Debug, Clone)]
pub struct DecodedRaw {
    /// Active-area width in pixels.
    pub width: u32,
    /// Active-area height in pixels.
    pub height: u32,
    /// Interleaved RGB f32, row-major, `width * height * 3` samples.
    ///
    /// Linear camera-native colour, black-subtracted, normalized so that a
    /// saturated sensor value maps to ≥ 1.0 (see the module docs for the
    /// exact formula). Values are unbounded; do not assume [0, 1].
    pub pixels: Vec<f32>,
    /// Capture and colour metadata extracted from the file.
    pub metadata: RawMetadata,
}

/// Metadata extracted from a raw file at decode time.
///
/// Every field is best-effort: `None` means the file did not carry the
/// information (or rawshift 0.1.1 does not parse it yet).
#[derive(Debug, Clone, PartialEq)]
pub struct RawMetadata {
    /// Camera manufacturer (e.g. "SONY").
    pub camera_make: Option<String>,
    /// Camera model (e.g. "ILCE-7RM5").
    pub camera_model: Option<String>,
    /// As-shot neutral in camera RGB, green-normalized (`g == 1.0`).
    ///
    /// This is the colour of a neutral surface under the scene illuminant;
    /// dividing each channel by it yields the as-shot white-balance gains.
    pub as_shot_neutral: Option<[f32; 3]>,
    /// XYZ→camera matrix (row-major rows), DNG convention.
    ///
    /// Dual-illuminant calibrations are interpolated at the as-shot CCT when
    /// possible (module docs). `None` when the file embeds no matrix and the
    /// model is not in rawshift's calibration database.
    pub xyz_to_camera: Option<[[f32; 3]; 3]>,
    /// EXIF orientation, 1–8. Defaults to 1 (no transform) when absent.
    /// Never baked into [`DecodedRaw::pixels`].
    pub orientation: u16,
    /// EXIF `DateTimeOriginal` exactly as stored
    /// (EXIF format `"YYYY:MM:DD HH:MM:SS"`).
    pub capture_time: Option<String>,
    /// ISO sensitivity.
    pub iso: Option<u32>,
    /// Exposure time in seconds.
    pub exposure_time: Option<f32>,
    /// F-number (aperture).
    pub f_number: Option<f32>,
    /// Focal length in millimetres.
    pub focal_length: Option<f32>,
    /// Lens model name.
    pub lens_model: Option<String>,
    /// Which optics-correction sources are present (docs/subsystems/pipeline.md stage 2).
    pub optics: OpticsMetadata,
}

/// Presence of embedded optics-correction metadata, per correction kind.
///
/// All `false` in v1: rawshift 0.1.1 exposes no optics metadata for ARW
/// (undecoded Sony MakerNote) and none on the DNG Bayer path (module docs).
/// The optics stage must emit its visible "stage skipped" warning whenever
/// the corresponding flag is `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpticsMetadata {
    /// Vignetting (lens shading) correction data is present.
    pub vignetting: bool,
    /// Lateral chromatic aberration correction data is present.
    pub chromatic_aberration: bool,
    /// Geometric distortion correction data is present.
    pub distortion: bool,
}

/// Errors surfaced by the decode stage.
///
/// Per-file and typed so the UI can report precisely why a file cannot be
/// opened (docs/subsystems/decode.md: "we surface a clear per-file error").
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The container was recognized but its pixel data uses a compression
    /// scheme rawshift cannot decode yet (e.g. uncompressed or lossy ARW —
    /// only Sony lossless type 7 is supported today).
    #[error("unsupported raw compression: {detail}")]
    UnsupportedCompression {
        /// Upstream description of the unsupported scheme.
        detail: String,
    },
    /// The file is not a supported raw format (or a supported container with
    /// an unsupported layout, e.g. pre-demosaiced LinearRaw DNG).
    #[error("unsupported raw format: {0}")]
    UnsupportedFormat(String),
    /// The file could not be read.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is damaged or rawshift failed to decode it.
    #[error("raw decode failed: {0}")]
    Decode(String),
}

/// Decodes a raw file to linear camera RGB f32 (module docs for the exact
/// pipeline and normalization formula).
pub fn decode_file(path: &Path) -> Result<DecodedRaw, DecodeError> {
    let file = File::open(path)?;
    let mut raw = RawFile::open(BufReader::new(file)).map_err(open_error)?;

    if raw.is_linear_raw_dng() {
        return Err(DecodeError::UnsupportedFormat(
            "LinearRaw DNG (pre-demosaiced, e.g. Apple ProRAW) is not supported \
             by the Bayer decode path"
                .to_string(),
        ));
    }

    let meta = raw.metadata();
    let mut cfa = raw.decode_raw().map_err(decode_error)?;

    // Pedestal subtraction (per CFA site, saturating at 0).
    apply_black_level(&mut cfa);

    let max_black = *cfa.black_levels().iter().max().expect("array is non-empty");
    let white_level = cfa.white_level();
    let effective_white = white_level.saturating_sub(max_black);
    if effective_white == 0 {
        return Err(DecodeError::Decode(format!(
            "invalid sensor levels: white level {white_level} <= black level {max_black}"
        )));
    }

    // Demosaic. Pinned to AMaZE (module docs); output is active-area sized.
    let rgb = DemosaicMethod::Bayer(BayerAlgorithm::Amaze)
        .to_demosaic()
        .demosaic(&cfa);
    let (width, height) = (rgb.width(), rgb.height());

    // Normalize u16 -> f32. Sequential, fixed order; one IEEE division per
    // sample — this loop IS the documented formula.
    let divisor = effective_white as f32;
    let pixels: Vec<f32> = rgb.data.iter().map(|&v| v as f32 / divisor).collect();

    Ok(DecodedRaw {
        width,
        height,
        pixels,
        metadata: extract_metadata(&meta),
    })
}

/// Extracts the embedded JPEG thumbnail, if the file carries one.
///
/// Returns `Ok(None)` when the format has no embedded thumbnail (or rawshift
/// does not extract it yet). The bytes are the JPEG stream exactly as stored.
pub fn extract_thumbnail(path: &Path) -> Result<Option<Vec<u8>>, DecodeError> {
    let file = File::open(path)?;
    let mut raw = RawFile::open(BufReader::new(file)).map_err(open_error)?;
    raw.thumbnail().map_err(decode_error)
}

/// Returns true when `path` has a raw extension this module can try to
/// decode (`.arw`, `.dng`; case-insensitive). Purely name-based — the
/// authoritative check is [`decode_file`] itself.
pub fn is_raw_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("arw") || ext.eq_ignore_ascii_case("dng"))
}

/// Maps errors from `RawFile::open` (format detection / container parsing).
fn open_error(err: RawError) -> DecodeError {
    match err {
        RawError::Io(e) => DecodeError::Io(e),
        RawError::Unsupported(msg) => DecodeError::UnsupportedFormat(msg),
        other => DecodeError::Decode(other.to_string()),
    }
}

/// Maps errors from pixel decoding, classifying unsupported-compression
/// reports (rawshift models them as untyped `Unsupported` strings — e.g.
/// "Sony Compressed (Type 8) not yet supported…", "Unsupported DNG strip
/// compression: …" — so classification is by message).
fn decode_error(err: RawError) -> DecodeError {
    match err {
        RawError::Io(e) => DecodeError::Io(e),
        RawError::Unsupported(msg) => {
            if msg.to_ascii_lowercase().contains("compress") {
                DecodeError::UnsupportedCompression { detail: msg }
            } else {
                DecodeError::UnsupportedFormat(msg)
            }
        }
        other => DecodeError::Decode(other.to_string()),
    }
}

/// Builds [`RawMetadata`] from rawshift's unified metadata.
fn extract_metadata(meta: &ImageMetadata) -> RawMetadata {
    RawMetadata {
        camera_make: non_empty(&meta.camera.make),
        camera_model: non_empty(&meta.camera.model),
        as_shot_neutral: green_normalized_neutral(meta.dng_color.as_shot_neutral),
        xyz_to_camera: resolve_xyz_to_camera(meta),
        orientation: meta.image.orientation.unwrap_or(1),
        capture_time: meta.datetime.datetime_original.clone(),
        iso: meta.exif.iso,
        exposure_time: rational_f32(meta.exif.exposure_time),
        f_number: rational_f32(meta.exif.f_number),
        focal_length: rational_f32(meta.exif.focal_length),
        lens_model: meta.camera.lens_model.clone(),
        // rawshift 0.1.1 exposes no optics metadata on this path (module docs).
        optics: OpticsMetadata::default(),
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn rational_f32(r: Option<URational>) -> Option<f32> {
    r.map(|r| r.to_f64() as f32).filter(|v| v.is_finite())
}

/// Green-normalizes a raw AsShotNeutral triple; `None` unless all three
/// components are strictly positive and finite.
fn green_normalized_neutral(neutral: Option<[f64; 3]>) -> Option<[f32; 3]> {
    let n = neutral?;
    if !n.iter().all(|&v| v.is_finite() && v > 0.0) {
        return None;
    }
    Some([(n[0] / n[1]) as f32, 1.0, (n[2] / n[1]) as f32])
}

/// Resolves the XYZ→camera matrix (module docs): embedded DNG matrices when
/// present, otherwise rawshift's per-model calibration database; dual
/// illuminants interpolated at the as-shot CCT when possible.
fn resolve_xyz_to_camera(meta: &ImageMetadata) -> Option<[[f32; 3]; 3]> {
    let dng = &meta.dng_color;
    let (m1, ill1, m2, ill2) = if dng.color_matrix_1.is_some() || dng.color_matrix_2.is_some() {
        (
            dng.color_matrix_1,
            dng.calibration_illuminant_1,
            dng.color_matrix_2,
            dng.calibration_illuminant_2,
        )
    } else {
        let model = meta.camera.model.trim();
        if model.is_empty() {
            return None;
        }
        let cal = find_camera_calibration(model)?;
        (
            cal.color_matrix_1,
            cal.illuminant_1,
            cal.color_matrix_2,
            cal.illuminant_2,
        )
    };

    match (m1, m2) {
        (Some(a), Some(b)) => {
            let interpolated = (|| {
                let cct1 = illuminant_cct(ill1?)?;
                let cct2 = illuminant_cct(ill2?)?;
                let neutral = meta.dng_color.as_shot_neutral?;
                if !neutral.iter().all(|&v| v.is_finite() && v > 0.0) {
                    return None;
                }
                let scene_cct = estimate_cct_from_as_shot_neutral(neutral);
                Some(interpolate_color_matrix(
                    &rows(a),
                    cct1,
                    &rows(b),
                    cct2,
                    scene_cct,
                ))
            })();
            // Without interpolation inputs, prefer matrix 2 (conventionally
            // the higher-CCT / D65 calibration in DNG practice).
            Some(rows_f32(interpolated.unwrap_or_else(|| rows(b))))
        }
        (None, Some(b)) => Some(rows_f32(rows(b))),
        (Some(a), None) => Some(rows_f32(rows(a))),
        (None, None) => None,
    }
}

/// CCT in kelvin for an EXIF `LightSource` calibration-illuminant code.
///
/// Table follows the DNG SDK (`dng_camera_profile`): standard illuminants at
/// their defined temperatures, fluorescent families at the midpoint of their
/// EXIF ranges. Unknown codes yield `None` (interpolation is then skipped).
fn illuminant_cct(light_source: u16) -> Option<f32> {
    match light_source {
        3 | 17 => Some(2850.0),              // Tungsten, Standard Light A
        24 => Some(3200.0),                  // ISO studio tungsten
        15 => Some(3450.0),                  // White fluorescent (3200–3700 K)
        2 | 14 => Some(4200.0),              // Fluorescent, cool white (3900–4500 K)
        13 => Some(5000.0),                  // Day white fluorescent (4600–5400 K)
        23 => Some(5000.0),                  // D50
        1 | 4 | 9 | 18 | 20 => Some(5500.0), // Daylight, flash, fine weather, Std B, D55
        12 => Some(6400.0),                  // Daylight fluorescent (5700–7100 K)
        10 | 19 | 21 => Some(6500.0),        // Cloudy, Standard Light C, D65
        11 | 22 => Some(7500.0),             // Shade, D75
        _ => None,
    }
}

/// `[f64; 9]` row-major → 3×3 rows.
fn rows(m: [f64; 9]) -> [[f64; 3]; 3] {
    [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]]
}

/// 3×3 f64 rows → 3×3 f32 rows.
fn rows_f32(m: [[f64; 3]; 3]) -> [[f32; 3]; 3] {
    m.map(|row| row.map(|v| v as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawshift_image::data::cameras::light_source;

    #[test]
    fn raw_candidate_by_extension_case_insensitive() {
        assert!(is_raw_candidate(Path::new("a/b/IMG_0001.ARW")));
        assert!(is_raw_candidate(Path::new("shot.arw")));
        assert!(is_raw_candidate(Path::new("shot.DnG")));
        assert!(!is_raw_candidate(Path::new("shot.jpg")));
        assert!(!is_raw_candidate(Path::new("shot.arw.xmp")));
        assert!(!is_raw_candidate(Path::new("arw")));
    }

    #[test]
    fn illuminant_cct_covers_dng_standard_pairs() {
        assert_eq!(illuminant_cct(light_source::STANDARD_LIGHT_A), Some(2850.0));
        assert_eq!(illuminant_cct(light_source::D50), Some(5000.0));
        assert_eq!(illuminant_cct(light_source::D55), Some(5500.0));
        assert_eq!(illuminant_cct(light_source::D65), Some(6500.0));
        assert_eq!(illuminant_cct(light_source::D75), Some(7500.0));
        assert_eq!(illuminant_cct(0), None);
        assert_eq!(illuminant_cct(255), None);
    }

    #[test]
    fn neutral_is_green_normalized_and_validated() {
        assert_eq!(
            green_normalized_neutral(Some([0.5, 2.0, 1.0])),
            Some([0.25, 1.0, 0.5])
        );
        assert_eq!(green_normalized_neutral(Some([0.5, 0.0, 1.0])), None);
        assert_eq!(green_normalized_neutral(Some([-0.5, 1.0, 1.0])), None);
        assert_eq!(green_normalized_neutral(None), None);
    }

    #[test]
    fn compression_errors_are_classified() {
        let err = decode_error(RawError::Unsupported(
            "Sony Compressed (Type 8) not yet supported. Only Uncompressed/LJPEG (Type 7) \
             is supported."
                .to_string(),
        ));
        assert!(matches!(err, DecodeError::UnsupportedCompression { .. }));

        let err = decode_error(RawError::Unsupported(
            "Unsupported DNG strip compression: 34892".to_string(),
        ));
        assert!(matches!(err, DecodeError::UnsupportedCompression { .. }));

        let err = decode_error(RawError::Unsupported("No raw data found".to_string()));
        assert!(matches!(err, DecodeError::UnsupportedFormat(_)));
    }

    fn meta_with_matrices(
        m1: Option<[f64; 9]>,
        ill1: Option<u16>,
        m2: Option<[f64; 9]>,
        ill2: Option<u16>,
        neutral: Option<[f64; 3]>,
    ) -> ImageMetadata {
        ImageMetadata {
            dng_color: rawshift_image::core::metadata::DngColorInfo {
                color_matrix_1: m1,
                calibration_illuminant_1: ill1,
                color_matrix_2: m2,
                calibration_illuminant_2: ill2,
                as_shot_neutral: neutral,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn meta_with_model(model: &str) -> ImageMetadata {
        ImageMetadata {
            camera: rawshift_image::core::metadata::CameraInfo {
                model: model.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    const M_ONES: [f64; 9] = [1.0; 9];
    const M_THREES: [f64; 9] = [3.0; 9];

    #[test]
    fn single_matrix_is_used_verbatim() {
        let meta = meta_with_matrices(None, None, Some(M_THREES), Some(21), None);
        assert_eq!(resolve_xyz_to_camera(&meta), Some([[3.0f32; 3]; 3]));

        let meta = meta_with_matrices(Some(M_ONES), Some(17), None, None, None);
        assert_eq!(resolve_xyz_to_camera(&meta), Some([[1.0f32; 3]; 3]));
    }

    #[test]
    fn dual_matrices_without_neutral_prefer_matrix_2() {
        let meta = meta_with_matrices(Some(M_ONES), Some(17), Some(M_THREES), Some(21), None);
        assert_eq!(resolve_xyz_to_camera(&meta), Some([[3.0f32; 3]; 3]));
    }

    #[test]
    fn dual_matrices_interpolate_at_as_shot_cct() {
        // Very warm neutral (B/R tiny) → scene CCT clamps toward illuminant 1
        // → result near matrix 1.
        let warm = meta_with_matrices(
            Some(M_ONES),
            Some(17),
            Some(M_THREES),
            Some(21),
            Some([1.0, 1.0, 0.001]),
        );
        let m = resolve_xyz_to_camera(&warm).unwrap();
        assert!(m[0][0] < 1.5, "warm scene should sit near matrix 1: {m:?}");

        // Very cool neutral (B/R large → CCT clamps at 10000 K > D65) → matrix 2.
        let cool = meta_with_matrices(
            Some(M_ONES),
            Some(17),
            Some(M_THREES),
            Some(21),
            Some([0.3, 1.0, 0.9]),
        );
        let m = resolve_xyz_to_camera(&cool).unwrap();
        assert_eq!(m, [[3.0f32; 3]; 3]);
    }

    #[test]
    fn falls_back_to_camera_database_by_model() {
        let meta = meta_with_model("ILCE-7RM5");
        let m = resolve_xyz_to_camera(&meta).expect("ILCE-7RM5 is in rawshift's database");
        // Spot-check against the database's ColorMatrix2 leading coefficient.
        assert!((m[0][0] - 0.8200).abs() < 1e-6);

        let unknown = meta_with_model("Definitely Not A Camera");
        assert_eq!(resolve_xyz_to_camera(&unknown), None);
        assert_eq!(resolve_xyz_to_camera(&ImageMetadata::default()), None);
    }
}
