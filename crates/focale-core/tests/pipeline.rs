//! End-to-end pipeline v1 integration: decode the committed fixture, run a
//! rich edit through every stage, and pin the output as a regression golden
//! (PRD §10: pipeline-version regression suite).

use focale_core::decode::decode_file;
use focale_core::masks::*;
use focale_core::params::EditState;
use focale_core::params::color::GradingWheel;
use focale_core::params::geometry::CropRect;
use focale_core::params::local::{LocalAdjustment, LocalParams};
use focale_core::params::retouch::{RetouchMode, RetouchStroke};
use focale_core::params::tone::{CurvePoint, ToneCurve};
use focale_core::params::white_balance::WhiteBalanceParams;
use focale_core::pipeline::{RenderInput, RenderWarning, render};
use sha2::{Digest, Sha256};

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic.dng")
}

/// An edit state exercising every stage with non-default values.
fn rich_edit() -> EditState {
    let mut edit = EditState::default();
    edit.white_balance = WhiteBalanceParams::Temperature {
        kelvin: 4800.0,
        tint: 12.0,
    };
    edit.tone.exposure = 0.6;
    edit.tone.contrast = 20.0;
    edit.tone.highlights = -35.0;
    edit.tone.shadows = 25.0;
    edit.tone.whites = 10.0;
    edit.tone.blacks = -8.0;
    edit.tone.curve = ToneCurve {
        points: vec![
            CurvePoint { x: 0.0, y: 0.02 },
            CurvePoint { x: 0.45, y: 0.42 },
            CurvePoint { x: 1.0, y: 0.98 },
        ],
    };
    edit.color.vibrance = 18.0;
    edit.color.saturation = -6.0;
    edit.color.hsl.hue[0] = 30.0;
    edit.color.hsl.saturation[5] = -40.0;
    edit.color.grading.shadows = GradingWheel {
        hue: 220.0,
        saturation: 25.0,
        luminance: -5.0,
    };
    edit.local.push(LocalAdjustment {
        enabled: true,
        mask: MaskGroup {
            name: "test".into(),
            components: vec![
                MaskComponent {
                    op: MaskOp::Add,
                    invert: false,
                    feather: 0.02,
                    density: 1.0,
                    shape: MaskShape::Radial(RadialGradientMask {
                        center: [0.5, 0.5],
                        radius: [0.4, 0.3],
                        rotation: 15.0,
                        falloff: 0.5,
                    }),
                },
                MaskComponent {
                    op: MaskOp::Subtract,
                    invert: false,
                    feather: 0.0,
                    density: 0.8,
                    shape: MaskShape::Linear(LinearGradientMask {
                        start: [0.0, 0.8],
                        end: [0.0, 1.0],
                    }),
                },
            ],
        },
        adjustments: LocalParams {
            exposure: 0.8,
            saturation: 15.0,
            ..Default::default()
        },
    });
    edit.detail.sharpen.amount = 55.0;
    edit.detail.sharpen.radius = 1.2;
    edit.detail.noise_reduction.luminance = 20.0;
    edit.detail.noise_reduction.chroma = 25.0;
    edit.retouch.strokes.push(RetouchStroke {
        mode: RetouchMode::Heal,
        radius: 0.06,
        feather: 0.5,
        opacity: 1.0,
        dest: vec![[0.3, 0.3]],
        source_offset: [0.2, 0.1],
    });
    edit.geometry.rotate = 2.5;
    edit.geometry.perspective.vertical = 10.0;
    edit.geometry.crop = Some(CropRect {
        x0: 0.1,
        y0: 0.1,
        x1: 0.9,
        y1: 0.85,
    });
    edit.finishing.vignette.amount = -30.0;
    edit.finishing.grain.amount = 20.0;
    edit.finishing.grain.seed = 7;
    edit
}

fn working_hash(edit: &EditState, scale: f32) -> (String, u32, u32) {
    let decoded = decode_file(&fixture()).expect("fixture decodes");
    let input = RenderInput {
        decoded: &decoded,
        edit,
        scale,
    };
    let out = render(&input, 1).expect("v1 supported");
    let mut hasher = Sha256::new();
    for v in out.image.data() {
        hasher.update(v.to_le_bytes());
    }
    (
        format!("{:x}", hasher.finalize()),
        out.image.width(),
        out.image.height(),
    )
}

#[test]
fn default_edit_reports_optics_warning() {
    let decoded = decode_file(&fixture()).expect("fixture decodes");
    let edit = EditState::default();
    let out = render(
        &RenderInput {
            decoded: &decoded,
            edit: &edit,
            scale: 1.0,
        },
        1,
    )
    .unwrap();
    assert!(out.warnings.contains(&RenderWarning::OpticsMetadataMissing));
    // The synthetic fixture embeds ColorMatrix1/2, so no matrix warning.
    assert!(!out.warnings.contains(&RenderWarning::CameraMatrixMissing));
}

#[test]
fn unsupported_version_is_rejected() {
    let decoded = decode_file(&fixture()).expect("fixture decodes");
    let edit = EditState::default();
    assert!(
        render(
            &RenderInput {
                decoded: &decoded,
                edit: &edit,
                scale: 1.0,
            },
            focale_core::PIPELINE_VERSION + 1,
        )
        .is_err()
    );
}

#[test]
fn rich_edit_renders_deterministically() {
    let edit = rich_edit();
    let (h1, w, h) = working_hash(&edit, 1.0);
    let (h2, ..) = working_hash(&edit, 1.0);
    assert_eq!(h1, h2, "two full renders must be bit-identical");
    // Crop of the oriented frame: fixture is 60x44 active, orientation 6
    // (90° CW) makes the canvas 44x60; crop [0.1..0.9]x[0.1..0.85] rounds to
    // x 4..40, y 6..51.
    assert_eq!((w, h), (36, 45));
}

/// Frozen pipeline-v1 regression golden. If this hash ever changes, v1
/// output changed — that is a PRD §2.2 violation unless the change is a new
/// pipeline version. Bless intentionally-new goldens only before the first
/// release, with FOCALE_BLESS=1.
#[test]
fn rich_edit_matches_frozen_golden() {
    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pipeline-v1-golden.txt");
    let (hash, w, h) = working_hash(&rich_edit(), 1.0);
    let line = format!("{hash} {w}x{h}\n");
    if std::env::var_os("FOCALE_BLESS").is_some() {
        std::fs::write(&golden_path, &line).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden_path)
        .expect("golden file present (generate once with FOCALE_BLESS=1)");
    assert_eq!(line, expected, "pipeline v1 output changed — see PRD §2.2");
}
