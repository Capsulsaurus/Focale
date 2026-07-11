//! Determinism fixtures for CI (docs/architecture.md §11).
//!
//! `determinism.fcl` is the committed sidecar the cross-architecture CI
//! renders on x86_64 and aarch64 (.github/workflows/determinism.yml). The
//! tests below keep the fixture honest: its bytes must match what the
//! current schema serializes, and rendering + encoding it twice must be
//! bit-identical in-process.

use focale_core::masks::*;
use focale_core::params::EditState;
use focale_core::params::color::GradingWheel;
use focale_core::params::geometry::CropRect;
use focale_core::params::local::{LocalAdjustment, LocalParams};
use focale_core::params::retouch::{RetouchMode, RetouchStroke};
use focale_core::params::tone::{CurvePoint, ToneCurve};
use focale_core::params::white_balance::WhiteBalanceParams;
use focale_sidecar::SidecarDoc;

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn raw_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../focale-core/tests/fixtures/synthetic.dng")
}

/// The canonical determinism edit: exercises every stage.
#[allow(clippy::field_reassign_with_default)]
fn determinism_doc() -> SidecarDoc {
    let mut edit = EditState::default();
    edit.white_balance = WhiteBalanceParams::Temperature {
        kelvin: 5200.0,
        tint: -8.0,
    };
    edit.tone.exposure = 0.4;
    edit.tone.contrast = 15.0;
    edit.tone.highlights = -30.0;
    edit.tone.shadows = 20.0;
    edit.tone.curve = ToneCurve {
        points: vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.55 },
            CurvePoint { x: 1.0, y: 1.0 },
        ],
    };
    edit.color.vibrance = 12.0;
    edit.color.hsl.saturation[1] = 25.0;
    edit.color.grading.highlights = GradingWheel {
        hue: 45.0,
        saturation: 15.0,
        luminance: 0.0,
    };
    edit.local.push(LocalAdjustment {
        enabled: true,
        mask: MaskGroup {
            name: "sky-ish".into(),
            components: vec![MaskComponent {
                op: MaskOp::Add,
                invert: false,
                feather: 0.01,
                density: 0.9,
                shape: MaskShape::Linear(LinearGradientMask {
                    start: [0.5, 0.0],
                    end: [0.5, 0.55],
                }),
            }],
        },
        adjustments: LocalParams {
            exposure: -0.7,
            saturation: 10.0,
            ..Default::default()
        },
    });
    edit.detail.sharpen.amount = 45.0;
    edit.detail.noise_reduction.luminance = 15.0;
    edit.retouch.strokes.push(RetouchStroke {
        mode: RetouchMode::Clone,
        radius: 0.05,
        feather: 0.4,
        opacity: 0.9,
        dest: vec![[0.25, 0.6], [0.3, 0.62]],
        source_offset: [0.15, -0.1],
    });
    edit.geometry.rotate = -1.5;
    edit.geometry.crop = Some(CropRect {
        x0: 0.05,
        y0: 0.05,
        x1: 0.95,
        y1: 0.9,
    });
    edit.finishing.vignette.amount = -25.0;
    edit.finishing.grain.amount = 15.0;
    edit.finishing.grain.seed = 42;

    let mut doc = SidecarDoc::new_default(focale_core::PIPELINE_VERSION);
    doc.edit = edit;
    doc
}

/// The committed fixture must match the in-code canonical doc byte-for-byte
/// (regenerate intentionally with FOCALE_BLESS=1).
#[test]
fn fixture_matches_canonical_document() {
    let path = fixture_dir().join("determinism.fcl");
    let bytes = determinism_doc().save_to_bytes().unwrap();
    if std::env::var_os("FOCALE_BLESS").is_some() {
        std::fs::create_dir_all(fixture_dir()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        return;
    }
    let committed = std::fs::read(&path).expect("fixture present (FOCALE_BLESS=1 to create)");
    assert_eq!(bytes, committed, "determinism.fcl drifted from the schema");
}

/// Full render + encode twice must be bit-identical in-process for every
/// format (the CI workflow proves the same across architectures).
#[test]
fn render_and_encode_twice_bit_identical() {
    use focale_sidecar::schema::{
        ExportColor, ExportFormat, ExportGamut, ExportRecipe, TiffCompression,
    };
    let doc = determinism_doc();
    let render = || {
        let decoded = focale_core::decode::decode_file(&raw_fixture()).unwrap();
        let input = focale_core::pipeline::RenderInput {
            decoded: &decoded,
            edit: &doc.edit,
            scale: 1.0,
        };
        focale_core::pipeline::render(&input, doc.pipeline_version)
            .unwrap()
            .image
    };
    let a = render();
    let b = render();
    assert_eq!(a.data(), b.data(), "pipeline output must be reproducible");

    for format in [
        ExportFormat::Tiff16 {
            compression: TiffCompression::Deflate,
        },
        ExportFormat::Png { bit_depth: 16 },
        ExportFormat::Jpeg { quality: 92 },
        ExportFormat::JpegXl {
            distance: 1.0,
            bit_depth: 16,
        },
        ExportFormat::Avif {
            quality: 80,
            bit_depth: 10,
        },
    ] {
        let recipe = ExportRecipe {
            name: "determinism".into(),
            format,
            color: ExportColor {
                gamut: ExportGamut::DisplayP3,
            },
            hdr: None,
            resize: None,
        };
        let e1 = focale_export::encode(&a, &recipe).unwrap();
        let e2 = focale_export::encode(&b, &recipe).unwrap();
        assert_eq!(e1, e2, "{:?} bytes must be reproducible", recipe.format);
    }
}
