//! Integration tests for `focale_core::decode` against the committed
//! synthetic DNG fixture (`tests/fixtures/synthetic.dng`, built by
//! `tests/gen_fixture.rs` — see that file for how to regenerate it).

use std::path::PathBuf;

use focale_core::decode::{DecodeError, decode_file, extract_thumbnail, is_raw_candidate};
use sha2::{Digest, Sha256};

/// SHA-256 of the decoded f32 pixel buffer (little-endian bytes) for the
/// committed fixture. This is the decode stage's determinism golden: any
/// change to decode/black-level/demosaic/normalization output — across
/// machines, architectures and thread counts — fails this test and requires
/// a pipeline version bump (AGENTS.md).
const GOLDEN_PIXEL_SHA256: &str =
    "865512c31548fa717212073d581592d6547f9636df1374bd251cbee381a6c2ab";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("synthetic.dng")
}

fn pixel_sha256(pixels: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for v in pixels {
        hasher.update(v.to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").unwrap();
    }
    hex
}

#[test]
fn decodes_fixture_dimensions_and_range() {
    let decoded = decode_file(&fixture()).expect("fixture must decode");

    // Active area of the 64×48 sensor is 60×44 (2-pixel border).
    assert_eq!(decoded.width, 60);
    assert_eq!(decoded.height, 44);
    assert_eq!(
        decoded.pixels.len(),
        60 * 44 * 3,
        "interleaved RGB f32 buffer"
    );

    // Linear camera RGB: finite and non-negative, with real signal present.
    // AMaZE may overshoot past the brightest input sample (it clamps at the
    // sensor white level, 16383), so the ceiling after normalization by the
    // effective white (16383 − 512 = 15871) is 16383/15871, not 1.0 — the
    // buffer is documented as unbounded.
    assert!(decoded.pixels.iter().all(|v| v.is_finite() && *v >= 0.0));
    let ceiling = 16383.0f32 / 15871.0;
    assert!(decoded.pixels.iter().all(|v| *v <= ceiling));
    let max = decoded.pixels.iter().cloned().fold(0.0f32, f32::max);
    assert!(max > 0.5, "gradient should reach past mid-grey, got {max}");
}

#[test]
fn decoded_pixels_match_determinism_golden() {
    let decoded = decode_file(&fixture()).expect("fixture must decode");
    assert_eq!(
        pixel_sha256(&decoded.pixels),
        GOLDEN_PIXEL_SHA256,
        "decode output changed — this requires a pipeline version bump"
    );
}

#[test]
fn extracts_fixture_metadata() {
    let m = decode_file(&fixture())
        .expect("fixture must decode")
        .metadata;

    assert_eq!(m.camera_make.as_deref(), Some("Focale"));
    assert_eq!(m.camera_model.as_deref(), Some("Synthetic RGGB"));
    assert_eq!(m.orientation, 6);
    assert_eq!(m.capture_time.as_deref(), Some("2026:01:15 12:00:00"));
    assert_eq!(m.iso, Some(100));
    assert_eq!(m.lens_model.as_deref(), Some("Focale Test 35mm F2.8"));

    let exposure = m.exposure_time.expect("exposure time present");
    assert!((exposure - 1.0 / 125.0).abs() < 1e-6);
    let f_number = m.f_number.expect("f-number present");
    assert!((f_number - 2.8).abs() < 1e-6);
    let focal = m.focal_length.expect("focal length present");
    assert!((focal - 35.0).abs() < 1e-6);

    // AsShotNeutral was written green-normalized: [0.473, 1.0, 0.624].
    let neutral = m.as_shot_neutral.expect("as-shot neutral present");
    assert!((neutral[0] - 0.473).abs() < 1e-6);
    assert!((neutral[1] - 1.0).abs() < 1e-6);
    assert!((neutral[2] - 0.624).abs() < 1e-6);

    // Dual-illuminant matrices must interpolate: every coefficient lies
    // within the (elementwise) envelope of ColorMatrix1 and ColorMatrix2.
    let cm1: [f32; 9] = [
        0.7374, -0.2389, -0.0551, -0.5435, 1.3162, 0.2519, -0.1006, 0.1795, 0.6552,
    ];
    let cm2: [f32; 9] = [
        0.8200, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
    ];
    let m3 = m.xyz_to_camera.expect("colour matrix present");
    for i in 0..9 {
        let v = m3[i / 3][i % 3];
        let (lo, hi) = (cm1[i].min(cm2[i]), cm1[i].max(cm2[i]));
        assert!(
            (lo - 1e-4..=hi + 1e-4).contains(&v),
            "coefficient {i} = {v} outside [{lo}, {hi}]"
        );
    }

    // rawshift 0.1.1 exposes no optics metadata (docs/subsystems/optics.md);
    // reported honestly as absent.
    assert!(!m.optics.vignetting);
    assert!(!m.optics.chromatic_aberration);
    assert!(!m.optics.distortion);
}

#[test]
fn fixture_has_no_embedded_thumbnail() {
    // The synthetic fixture carries no JPEG preview; the call must succeed
    // and report that, not error.
    let thumb = extract_thumbnail(&fixture()).expect("thumbnail extraction must not fail");
    assert_eq!(thumb, None);
}

#[test]
fn missing_file_is_io_error() {
    let err = decode_file(&fixture().with_file_name("nope.dng")).unwrap_err();
    assert!(matches!(err, DecodeError::Io(_)));
}

#[test]
fn non_raw_bytes_are_unsupported_format() {
    let dir = std::env::temp_dir().join(format!("focale-decode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("not-a-raw.dng");
    std::fs::write(&path, b"this is definitely not a TIFF container header").unwrap();

    let err = decode_file(&path).unwrap_err();
    assert!(
        matches!(err, DecodeError::UnsupportedFormat(_)),
        "expected UnsupportedFormat, got {err:?}"
    );

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[test]
fn raw_candidate_matches_fixture() {
    assert!(is_raw_candidate(&fixture()));
    assert!(!is_raw_candidate(&fixture().with_extension("fcl")));
}
