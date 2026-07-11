//! Generator for `tests/fixtures/synthetic.dng`.
//!
//! The fixture is a tiny (64×48, ~8 KB) hand-built Bayer-CFA DNG written with
//! rawshift's own TIFF writer. rawshift's high-level DNG *encoder*
//! (`export_dng`) writes demosaiced RGB files (PhotometricInterpretation = 2)
//! that its own DNG *decoder* rejects (it only accepts CFA 32803 / LinearRaw
//! 34892), so the fixture is assembled tag-by-tag instead — uncompressed
//! 16-bit CFA strips plus the colour-calibration, level, active-area and EXIF
//! tags the decode module consumes. It decodes through the public
//! `RawFile::open` path.
//!
//! The generating test is `#[ignore]`d so normal runs never rewrite the
//! committed fixture. Regenerate (then re-pin the golden hash in
//! `tests/decode.rs` if pixels changed) with:
//!
//! ```text
//! cargo test -p focale-core --test gen_fixture -- --ignored
//! ```

use std::io::Cursor;
use std::path::PathBuf;

use rawshift_image::tiff::writer::{IfdEntry, TiffWriter};
use rawshift_image::tiff::{ByteOrder, TiffTag};

/// Full sensor width of the synthetic image.
const WIDTH: u32 = 64;
/// Full sensor height of the synthetic image.
const HEIGHT: u32 = 48;
/// Per-CFA-site black level (pedestal).
const BLACK_LEVEL: u32 = 512;
/// Sensor saturation level (14-bit style, stored as 16-bit samples).
const WHITE_LEVEL: u32 = 16383;

/// Deterministic synthetic CFA data: a smooth diagonal gradient sitting on
/// the pedestal, everywhere below the white level. Pure integer arithmetic —
/// identical on every platform.
fn synthetic_cfa() -> Vec<u16> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut data = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let v = BLACK_LEVEL + ((x as u32 * 211 + y as u32 * 307) * 13) % 15000;
            data.push(v.min(WHITE_LEVEL) as u16);
        }
    }
    data
}

/// Builds the complete DNG byte stream in memory.
fn build_synthetic_dng() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let mut writer = TiffWriter::new(&mut buf, ByteOrder::LittleEndian);
    writer.write_header().unwrap();

    // Pixel data: one uncompressed strip of 16-bit samples. The writer method
    // is named for RGB but writes any &[u16] verbatim in file byte order.
    let cfa = synthetic_cfa();
    let (strip_offset, strip_bytes) = writer.write_image_strip_rgb16(&cfa).unwrap();

    // EXIF sub-IFD, written before IFD0 so its offset is known.
    let mut exif_entries = vec![
        IfdEntry::short(TiffTag::ISOSpeedRatings, 100),
        IfdEntry::rational(TiffTag::ExposureTime, 1, 125),
        IfdEntry::rational(TiffTag::FNumber, 28, 10),
        IfdEntry::rational(TiffTag::FocalLength, 35, 1),
        IfdEntry::ascii(TiffTag::DateTimeOriginal, "2026:01:15 12:00:00"),
        IfdEntry::ascii(TiffTag::LensModel, "Focale Test 35mm F2.8"),
    ];
    let exif_offset = writer.write_ifd(&mut exif_entries, 0).unwrap();

    // Dual-illuminant XYZ→camera calibration (plausible, invertible values;
    // matrix 2 is Sony ILCE-7RM5's D65 matrix from rawshift's database).
    let cm1: [f64; 9] = [
        0.7374, -0.2389, -0.0551, -0.5435, 1.3162, 0.2519, -0.1006, 0.1795, 0.6552,
    ];
    let cm2: [f64; 9] = [
        0.8200, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
    ];
    let to_srationals = |m: &[f64; 9]| -> Vec<(i32, i32)> {
        m.iter()
            .map(|&v| ((v * 10000.0).round() as i32, 10000))
            .collect()
    };

    let mut ifd0_entries = vec![
        IfdEntry::long(TiffTag::ImageWidth, WIDTH),
        IfdEntry::long(TiffTag::ImageLength, HEIGHT),
        IfdEntry::short(TiffTag::BitsPerSample, 16),
        IfdEntry::short(TiffTag::Compression, 1), // uncompressed
        IfdEntry::short(TiffTag::PhotometricInterpretation, 32803), // CFA
        IfdEntry::ascii(TiffTag::Make, "Focale"),
        IfdEntry::ascii(TiffTag::Model, "Synthetic RGGB"),
        IfdEntry::short(TiffTag::Orientation, 6),
        IfdEntry::short(TiffTag::SamplesPerPixel, 1),
        IfdEntry::long(TiffTag::RowsPerStrip, HEIGHT),
        IfdEntry::long(TiffTag::StripOffsets, strip_offset as u32),
        IfdEntry::long(TiffTag::StripByteCounts, strip_bytes as u32),
        IfdEntry::long(TiffTag::ExifIFDPointer, exif_offset as u32),
        IfdEntry::bytes(TiffTag::DNGVersion, &[1, 4, 0, 0]),
        IfdEntry::ascii(TiffTag::UniqueCameraModel, "Focale Synthetic RGGB"),
        IfdEntry::bytes(TiffTag::CFAPattern, &[0, 1, 1, 2]), // RGGB
        IfdEntry::longs(TiffTag::BlackLevel, &[BLACK_LEVEL; 4]),
        IfdEntry::longs(TiffTag::WhiteLevel, &[WHITE_LEVEL]),
        // ActiveArea is [top, left, bottom, right]: a 60×44 window with a
        // 2-pixel border, so tests exercise active-area cropping.
        IfdEntry::longs(TiffTag::ActiveArea, &[2, 2, 46, 62]),
        IfdEntry::srationals(TiffTag::ColorMatrix1, &to_srationals(&cm1)),
        IfdEntry::short(TiffTag::CalibrationIlluminant1, 17), // Standard Light A
        IfdEntry::srationals(TiffTag::ColorMatrix2, &to_srationals(&cm2)),
        IfdEntry::short(TiffTag::CalibrationIlluminant2, 21), // D65
        // AsShotNeutral (green-normalized): warm-ish daylight.
        IfdEntry::rationals(
            TiffTag::AsShotNeutral,
            &[(473, 1000), (1000, 1000), (624, 1000)],
        ),
    ];
    let ifd0_offset = writer.write_ifd(&mut ifd0_entries, 0).unwrap();
    writer.update_ifd0_offset(ifd0_offset as u32).unwrap();

    buf.into_inner()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("synthetic.dng")
}

/// Regenerates the committed fixture. `#[ignore]`d — run explicitly (module
/// docs). Verifies the bytes round-trip through `decode_file` before writing.
#[test]
#[ignore = "regenerates the committed fixture; run explicitly"]
fn regenerate_synthetic_fixture() {
    let bytes = build_synthetic_dng();
    assert!(bytes.len() < 100_000, "fixture must stay under 100 KB");

    let path = fixture_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    // Prove the fixture decodes through the public path.
    let decoded = focale_core::decode::decode_file(&path).expect("fixture must decode");
    assert_eq!((decoded.width, decoded.height), (60, 44));
    println!("wrote {} ({} bytes)", path.display(), bytes.len());
}

/// Non-ignored guard: the in-memory builder output must always be decodable,
/// so a rawshift upgrade that breaks the fixture layout fails loudly here
/// even without regenerating.
#[test]
fn builder_output_is_decodable() {
    let bytes = build_synthetic_dng();
    let dir = std::env::temp_dir().join(format!("focale-gen-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("synthetic.dng");
    std::fs::write(&path, &bytes).unwrap();

    let decoded = focale_core::decode::decode_file(&path).expect("builder output must decode");
    assert_eq!((decoded.width, decoded.height), (60, 44));

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}
