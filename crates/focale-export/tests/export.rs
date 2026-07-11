//! Integration tests for the export encoders: signatures, determinism
//! (byte-identical repeat encodes, AVIF/rav1e especially), numeric probes
//! of the SDR and HDR-PQ pathways, and v1 capability rejections.

use std::io::Cursor;

use focale_core::color::{
    Gamut, REINHARD_WHITE_DEFAULT, map_to_gamut, pq_encode_sdr, srgb_encode,
    tonemap_reinhard_extended,
};
use focale_core::image::ImageRgbF32;
use focale_export::{ExportError, encode};
use focale_sidecar::schema::{
    ExportColor, ExportFormat, ExportGamut, ExportRecipe, GainMapOptions, HdrOptions, HdrTransfer,
    ResizeSpec, TiffCompression,
};

/// A 64×40 synthetic test card: neutral probe pixels, a gray ramp running
/// into HDR (> 1.0), saturated Rec.2020 primaries (out of sRGB gamut), an
/// unbounded/negative stripe, a strong highlight stripe, and a colour
/// gradient.
fn test_card() -> ImageRgbF32 {
    let mut img = ImageRgbF32::new(64, 40);
    for y in 0..40 {
        for x in 0..64 {
            let t = x as f32 / 63.0;
            let px = match y / 8 {
                0 => [2.0 * t, 2.0 * t, 2.0 * t], // gray ramp into HDR
                1 => [1.0, 0.0, 0.0],             // Rec.2020 red, out of sRGB
                2 => [-0.05, 1.2, 0.02],          // unbounded + negative
                3 => [8.0, 6.0, 4.0],             // strong highlight
                _ => [t, 0.5 * t, 1.0 - t],       // colour gradient
            };
            img.set_pixel(x, y, px);
        }
    }
    // Fixed numeric probes (see pathway tests below).
    img.set_pixel(0, 0, [0.5, 0.5, 0.5]);
    img.set_pixel(1, 0, [1.0, 1.0, 1.0]);
    img
}

fn recipe(format: ExportFormat) -> ExportRecipe {
    ExportRecipe {
        name: "test".into(),
        format,
        color: ExportColor::default(), // sRGB
        hdr: None,
        resize: None,
    }
}

fn hdr_recipe(format: ExportFormat, transfer: HdrTransfer) -> ExportRecipe {
    ExportRecipe {
        hdr: Some(HdrOptions {
            transfer,
            peak_nits: 1000.0,
            gain_map: None,
        }),
        ..recipe(format)
    }
}

fn all_sdr_formats() -> Vec<(&'static str, ExportFormat)> {
    vec![
        (
            "tiff",
            ExportFormat::Tiff16 {
                compression: TiffCompression::Deflate,
            },
        ),
        ("png16", ExportFormat::Png { bit_depth: 16 }),
        ("jpeg", ExportFormat::Jpeg { quality: 90 }),
        (
            "jxl",
            ExportFormat::JpegXl {
                distance: 1.0,
                bit_depth: 16,
            },
        ),
        (
            "avif",
            ExportFormat::Avif {
                quality: 80,
                bit_depth: 10,
            },
        ),
    ]
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Decodes a PNG, returning (width, height, bit_depth, raw image bytes).
fn decode_png(bytes: &[u8]) -> (u32, u32, u8, Vec<u8>) {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("readable PNG");
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).expect("decodable PNG");
    let depth = info.bit_depth as u8;
    buf.truncate(info.buffer_size());
    (info.width, info.height, depth, buf)
}

/// Reads one 16-bit sample (big-endian, per PNG) from decoded RGB data.
fn png16_sample(data: &[u8], width: u32, x: u32, y: u32, channel: usize) -> u16 {
    let i = ((y * width + x) as usize * 3 + channel) * 2;
    u16::from_be_bytes([data[i], data[i + 1]])
}

#[test]
fn every_format_encodes_with_correct_signature() {
    let img = test_card();
    for (name, format) in all_sdr_formats() {
        let bytes = encode(&img, &recipe(format)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!bytes.is_empty(), "{name}: empty output");
        match name {
            "tiff" => {
                let magic_le = [0x49, 0x49, 42, 0];
                let magic_be = [0x4D, 0x4D, 0, 42];
                assert!(
                    bytes[..4] == magic_le || bytes[..4] == magic_be,
                    "{name}: bad magic {:?}",
                    &bytes[..4]
                );
            }
            "png16" => {
                assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "{name}");
                assert!(contains(&bytes, b"iCCP"), "SDR PNG must carry iCCP");
                assert!(!contains(&bytes, b"cICP"), "SDR PNG must not carry cICP");
            }
            "jpeg" => {
                assert_eq!(&bytes[..2], &[0xFF, 0xD8], "{name}");
                assert!(
                    contains(&bytes, b"ICC_PROFILE\0"),
                    "JPEG must carry an ICC APP2 marker"
                );
            }
            "jxl" => {
                assert_eq!(&bytes[..2], &[0xFF, 0x0A], "{name}: naked JXL codestream");
            }
            "avif" => {
                assert_eq!(&bytes[4..8], b"ftyp", "{name}");
                assert_eq!(&bytes[8..12], b"avif", "{name}: major brand");
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn every_format_is_byte_deterministic() {
    let img = test_card();
    for (name, format) in all_sdr_formats() {
        let r = recipe(format);
        let a = encode(&img, &r).unwrap_or_else(|e| panic!("{name}: {e}"));
        let b = encode(&img, &r).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(a, b, "{name}: repeat encode produced different bytes");
    }
}

/// The critical determinism check: rav1e, run twice in-process with the
/// pinned single-threaded configuration, must reproduce identical bytes —
/// for both SDR and HDR (PQ) pathways.
#[test]
fn avif_encode_is_byte_deterministic() {
    let img = test_card();
    let sdr = recipe(ExportFormat::Avif {
        quality: 80,
        bit_depth: 10,
    });
    let a = encode(&img, &sdr).expect("avif sdr");
    let b = encode(&img, &sdr).expect("avif sdr");
    assert_eq!(a, b, "rav1e SDR output is not reproducible");

    let hdr = hdr_recipe(
        ExportFormat::Avif {
            quality: 80,
            bit_depth: 10,
        },
        HdrTransfer::Pq,
    );
    let a = encode(&img, &hdr).expect("avif hdr");
    let b = encode(&img, &hdr).expect("avif hdr");
    assert_eq!(a, b, "rav1e HDR output is not reproducible");
}

#[test]
fn tiff_is_actually_16_bit_with_icc() {
    let img = test_card();
    for compression in [
        TiffCompression::None,
        TiffCompression::Deflate,
        TiffCompression::Lzw,
    ] {
        let bytes = encode(&img, &recipe(ExportFormat::Tiff16 { compression })).expect("tiff");
        let mut decoder = tiff::decoder::Decoder::new(Cursor::new(&bytes)).expect("valid TIFF");
        assert_eq!(decoder.dimensions().unwrap(), (64, 40));
        assert_eq!(
            decoder.colortype().unwrap(),
            tiff::ColorType::RGB(16),
            "{compression:?}: BitsPerSample must be 16"
        );
        let icc = decoder
            .find_tag(tiff::tags::Tag::Unknown(34675))
            .expect("readable tags")
            .expect("ICC tag present");
        let icc_bytes = icc.into_u8_vec().expect("byte payload");
        assert_eq!(&icc_bytes[36..40], b"acsp", "embedded ICC is a profile");
        // Pixel data decodes as 16-bit.
        match decoder.read_image().expect("decodable image") {
            tiff::decoder::DecodingResult::U16(data) => {
                assert_eq!(data.len(), 64 * 40 * 3);
            }
            other => panic!("expected U16 data, got {other:?}"),
        }
    }
}

/// Validates the SDR pathway numerically: linear 0.5 neutral gray →
/// Reinhard tone map → (identity) gamut map → sRGB encode → 16-bit
/// quantize, cross-checked against an independent closed-form value.
#[test]
fn png16_probe_validates_sdr_pathway() {
    let img = test_card();
    let bytes = encode(&img, &recipe(ExportFormat::Png { bit_depth: 16 })).expect("png");
    let (w, h, depth, data) = decode_png(&bytes);
    assert_eq!((w, h, depth), (64, 40, 16));

    let expected = |linear: f32| -> u16 {
        let toned = tonemap_reinhard_extended([linear; 3], REINHARD_WHITE_DEFAULT);
        let mapped = map_to_gamut(toned, Gamut::Srgb);
        (srgb_encode(mapped[0]) * 65535.0 + 0.5).floor() as u16
    };
    for channel in 0..3 {
        let probe_half = png16_sample(&data, w, 0, 0, channel);
        assert_eq!(probe_half, expected(0.5), "channel {channel}");
        // Independent cross-check: tonemap(0.5) = 0.34375 exactly;
        // sRGB(0.34375) ≈ 0.62117 → ≈ 40711.
        assert!(
            (40650..=40780).contains(&probe_half),
            "linear 0.5 probe out of expected range: {probe_half}"
        );
        let probe_one = png16_sample(&data, w, 1, 0, channel);
        assert_eq!(probe_one, expected(1.0), "channel {channel}");
    }
}

/// Validates the HDR-PQ pathway numerically: linear 1.0 (SDR reference
/// white, 203 cd/m²) must land at PQ signal ≈ 0.5807 (ITU-R BT.2408).
#[test]
fn hdr_pq_png_probe_validates_pq_math() {
    let img = test_card();
    let r = hdr_recipe(ExportFormat::Png { bit_depth: 16 }, HdrTransfer::Pq);
    let bytes = encode(&img, &r).expect("hdr png");

    // cICP chunk present with payload (9, 16, 0, 1); no iCCP for HDR.
    assert!(contains(&bytes, &[b'c', b'I', b'C', b'P', 9, 16, 0, 1]));
    assert!(!contains(&bytes, b"iCCP"));

    let (w, _, depth, data) = decode_png(&bytes);
    assert_eq!(depth, 16);
    let probe = png16_sample(&data, w, 1, 0, 0); // linear 1.0 pixel
    let expected = (pq_encode_sdr(1.0) * 65535.0 + 0.5).floor() as u16;
    assert_eq!(probe, expected);
    // Independent cross-check: PQ(203 nits) ≈ 0.5807 → ≈ 38056.
    assert!(
        (37950..=38160).contains(&probe),
        "PQ probe out of expected range: {probe}"
    );
}

#[test]
fn hdr_hlg_png_signals_hlg_cicp() {
    let img = test_card();
    let r = hdr_recipe(ExportFormat::Png { bit_depth: 16 }, HdrTransfer::Hlg);
    let bytes = encode(&img, &r).expect("hlg png");
    assert!(contains(&bytes, &[b'c', b'I', b'C', b'P', 9, 18, 0, 1]));
}

#[test]
fn png8_and_resize_produce_expected_dimensions() {
    let img = test_card();
    let mut r = recipe(ExportFormat::Png { bit_depth: 8 });
    r.resize = Some(ResizeSpec { long_edge: 32 });
    let bytes = encode(&img, &r).expect("resized png");
    let (w, h, depth, _) = decode_png(&bytes);
    assert_eq!((w, h, depth), (32, 20, 8));
}

/// Either valid JPEG XL signature: the naked codestream (`FF 0A`) or the
/// ISOBMFF container (libjxl switches to it automatically when the
/// codestream needs level 10, e.g. 16-bit lossless).
fn assert_jxl_signature(bytes: &[u8]) {
    const CONTAINER: [u8; 12] = [
        0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A,
    ];
    assert!(
        bytes[..2] == [0xFF, 0x0A] || bytes[..12] == CONTAINER,
        "not a JXL signature: {:?}",
        &bytes[..12.min(bytes.len())]
    );
}

#[test]
fn jxl_lossless_and_hdr_encode() {
    let img = test_card();
    let lossless = recipe(ExportFormat::JpegXl {
        distance: 0.0,
        bit_depth: 16,
    });
    let bytes = encode(&img, &lossless).expect("lossless jxl");
    assert_jxl_signature(&bytes);

    let hdr = hdr_recipe(
        ExportFormat::JpegXl {
            distance: 1.0,
            bit_depth: 16,
        },
        HdrTransfer::Pq,
    );
    let bytes = encode(&img, &hdr).expect("hdr jxl");
    assert_jxl_signature(&bytes);
}

#[test]
fn hdr_avif_encodes_for_pq_and_hlg() {
    let img = test_card();
    for transfer in [HdrTransfer::Pq, HdrTransfer::Hlg] {
        let r = hdr_recipe(
            ExportFormat::Avif {
                quality: 70,
                bit_depth: 10,
            },
            transfer,
        );
        let bytes = encode(&img, &r).expect("hdr avif");
        assert_eq!(&bytes[4..8], b"ftyp");
    }
}

#[test]
fn every_gamut_exports_sdr_png_with_icc() {
    let img = test_card();
    for gamut in [
        ExportGamut::Srgb,
        ExportGamut::DisplayP3,
        ExportGamut::AdobeRgb,
        ExportGamut::Rec2020,
    ] {
        let mut r = recipe(ExportFormat::Png { bit_depth: 16 });
        r.color = ExportColor { gamut };
        let bytes = encode(&img, &r).unwrap_or_else(|e| panic!("{gamut:?}: {e}"));
        assert!(contains(&bytes, b"iCCP"), "{gamut:?}: iCCP missing");
    }
}

#[test]
fn gain_map_request_is_rejected() {
    let img = test_card();
    let mut r = hdr_recipe(ExportFormat::Png { bit_depth: 16 }, HdrTransfer::Pq);
    r.hdr.as_mut().unwrap().gain_map = Some(GainMapOptions::default());
    match encode(&img, &r) {
        Err(ExportError::Unsupported(msg)) => assert!(msg.contains("gain map")),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn v1_capability_gaps_are_rejected() {
    let img = test_card();
    // HDR TIFF and HDR JPEG are SDR-only in v1.
    for format in [
        ExportFormat::Tiff16 {
            compression: TiffCompression::Deflate,
        },
        ExportFormat::Jpeg { quality: 90 },
    ] {
        assert!(matches!(
            encode(&img, &hdr_recipe(format, HdrTransfer::Pq)),
            Err(ExportError::Unsupported(_))
        ));
    }
    // AVIF cannot signal Adobe RGB primaries.
    let mut r = recipe(ExportFormat::Avif {
        quality: 80,
        bit_depth: 10,
    });
    r.color = ExportColor {
        gamut: ExportGamut::AdobeRgb,
    };
    assert!(matches!(encode(&img, &r), Err(ExportError::Unsupported(_))));
}

#[test]
fn out_of_domain_recipes_are_rejected() {
    let img = test_card();
    let cases = [
        ExportFormat::Png { bit_depth: 12 },
        ExportFormat::Jpeg { quality: 0 },
        ExportFormat::Jpeg { quality: 101 },
        ExportFormat::JpegXl {
            distance: -1.0,
            bit_depth: 16,
        },
        ExportFormat::JpegXl {
            distance: 1.0,
            bit_depth: 10,
        },
        ExportFormat::Avif {
            quality: 0,
            bit_depth: 10,
        },
        ExportFormat::Avif {
            quality: 80,
            bit_depth: 9,
        },
    ];
    for format in cases {
        assert!(
            matches!(
                encode(&img, &recipe(format.clone())),
                Err(ExportError::InvalidRecipe(_))
            ),
            "{format:?} should be rejected"
        );
    }
}
