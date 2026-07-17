//! Golden-byte tests for the `.fcl` sidecar (architecture.md §13: golden-file tests
//! proving deterministic encoding).
//!
//! The canonical document below exercises every schema field. Its encoded
//! bytes are committed as `tests/golden/canonical.fcl`; any byte change is
//! a schema/encoding change and must be deliberate (new schema version).
//!
//! To regenerate the golden file after an intentional change:
//! `FOCALE_BLESS=1 cargo test -p focale-sidecar --test golden` and commit
//! the updated fixture together with the schema bump.

use std::path::Path;

use ciborium::Value;
use focale_core::masks::{
    BrushMask, BrushStroke, ColorRangeMask, LinearGradientMask, LuminanceRangeMask, MaskComponent,
    MaskGroup, MaskOp, MaskShape, PersonPart, RadialGradientMask, ResolvedMask, SegmentKind,
};
use focale_core::params::color::{ColorGrading, ColorParams, GradingWheel, HslBands};
use focale_core::params::detail::{NoiseReductionParams, SharpenMethod, SharpenParams};
use focale_core::params::finishing::{GrainParams, VignetteParams};
use focale_core::params::geometry::{CropRect, PerspectiveParams};
use focale_core::params::local::LocalParams;
use focale_core::params::retouch::{RetouchMode, RetouchStroke};
use focale_core::params::tone::{CurvePoint, ToneCurve};
use focale_core::params::{
    DetailParams, EditState, FinishingParams, GeometryParams, LocalAdjustment, OpticsParams,
    RetouchParams, ToneParams, WhiteBalanceParams,
};
use focale_sidecar::schema::{
    ExportColor, ExportFormat, ExportGamut, ExportRecipe, Flag, GainMapOptions, HdrOptions,
    HdrTransfer, LiveIndex, ResizeSpec, TiffCompression,
};
use focale_sidecar::{SCHEMA_VERSION, SidecarDoc, SidecarError, cde};

/// Builds a document with every schema field set to a non-default value.
fn canonical_doc() -> SidecarDoc {
    let optics = OpticsParams {
        enabled: true,
        vignetting: true,
        chromatic_aberration: false,
        distortion: true,
    };

    let white_balance = WhiteBalanceParams::Temperature {
        kelvin: 5600.0,
        tint: 12.5,
    };

    let tone = ToneParams {
        enabled: true,
        exposure: 0.75,
        contrast: 18.0,
        highlights: -42.0,
        shadows: 35.0,
        whites: -10.0,
        blacks: 6.0,
        curve: ToneCurve {
            points: vec![
                CurvePoint { x: 0.0, y: 0.03125 },
                CurvePoint { x: 0.25, y: 0.1875 },
                CurvePoint { x: 0.75, y: 0.8125 },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        },
    };

    let color = ColorParams {
        enabled: true,
        hsl: HslBands {
            hue: [4.0, -8.0, 0.0, 12.0, 0.0, -20.0, 0.0, 2.0],
            saturation: [0.0, 10.0, -15.0, 0.0, 25.0, 0.0, -5.0, 0.0],
            luminance: [-6.0, 0.0, 8.0, 0.0, 0.0, -12.0, 0.0, 3.0],
        },
        grading: ColorGrading {
            shadows: GradingWheel {
                hue: 220.0,
                saturation: 15.0,
                luminance: -5.0,
            },
            midtones: GradingWheel {
                hue: 40.0,
                saturation: 5.0,
                luminance: 0.0,
            },
            highlights: GradingWheel {
                hue: 55.0,
                saturation: 10.0,
                luminance: 4.0,
            },
            blending: 60.0,
            balance: -20.0,
        },
        vibrance: 22.0,
        saturation: -4.0,
    };

    // One local adjustment combining brush + radial + luminance-range masks
    // with different ops, and a second carrying a resolved AI mask.
    let local = vec![
        LocalAdjustment {
            enabled: true,
            mask: MaskGroup {
                name: "Sky burn".into(),
                components: vec![
                    MaskComponent {
                        op: MaskOp::Add,
                        invert: false,
                        feather: 0.02,
                        density: 1.0,
                        shape: MaskShape::Brush(BrushMask {
                            strokes: vec![
                                BrushStroke {
                                    erase: false,
                                    radius: 0.05,
                                    feather: 0.5,
                                    flow: 0.8,
                                    points: vec![[0.1, 0.2], [0.15, 0.25], [0.2, 0.28]],
                                },
                                BrushStroke {
                                    erase: true,
                                    radius: 0.02,
                                    feather: 0.25,
                                    flow: 1.0,
                                    points: vec![[0.18, 0.26]],
                                },
                            ],
                        }),
                    },
                    MaskComponent {
                        op: MaskOp::Subtract,
                        invert: true,
                        feather: 0.0,
                        density: 0.75,
                        shape: MaskShape::Radial(RadialGradientMask {
                            center: [0.5, 0.375],
                            radius: [0.25, 0.125],
                            rotation: 15.0,
                            falloff: 0.5,
                        }),
                    },
                    MaskComponent {
                        op: MaskOp::Intersect,
                        invert: false,
                        feather: 0.01,
                        density: 1.0,
                        shape: MaskShape::LuminanceRange(LuminanceRangeMask {
                            low: 0.625,
                            high: 1.0,
                            falloff: 0.25,
                        }),
                    },
                    MaskComponent {
                        op: MaskOp::Add,
                        invert: false,
                        feather: 0.0,
                        density: 0.5,
                        shape: MaskShape::Linear(LinearGradientMask {
                            start: [0.0, 0.125],
                            end: [0.0, 0.5],
                        }),
                    },
                    MaskComponent {
                        op: MaskOp::Intersect,
                        invert: false,
                        feather: 0.0,
                        density: 1.0,
                        shape: MaskShape::ColorRange(ColorRangeMask {
                            color: [0.125, 0.25, 0.75],
                            tolerance: 0.1,
                            falloff: 0.375,
                        }),
                    },
                ],
            },
            adjustments: LocalParams {
                exposure: -0.5,
                contrast: 10.0,
                highlights: -25.0,
                shadows: 5.0,
                whites: -8.0,
                blacks: 2.0,
                curve: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.5, y: 0.5625 },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ],
                },
                temperature: -12.0,
                tint: 4.0,
                tint_wheel: GradingWheel {
                    hue: 210.0,
                    saturation: 8.0,
                    luminance: 0.0,
                },
                vibrance: 15.0,
                saturation: -6.0,
            },
        },
        LocalAdjustment {
            enabled: false,
            mask: MaskGroup {
                name: "Lips".into(),
                components: vec![MaskComponent {
                    op: MaskOp::Add,
                    invert: false,
                    feather: 0.005,
                    density: 0.9,
                    shape: MaskShape::AiResolved(ResolvedMask {
                        kind: SegmentKind::PersonPart {
                            index: 0,
                            part: PersonPart::Lips,
                        },
                        width: 4,
                        height: 3,
                        deflate_bitmap: vec![0x78, 0x9c, 0x63, 0x64, 0x62, 0x66, 0x01, 0x00],
                    }),
                }],
            },
            adjustments: LocalParams {
                saturation: 12.0,
                ..LocalParams::default()
            },
        },
    ];

    let detail = DetailParams {
        enabled: true,
        sharpen: SharpenParams {
            method: SharpenMethod::Deconvolution,
            amount: 55.0,
            radius: 0.75,
            masking: 20.0,
        },
        noise_reduction: NoiseReductionParams {
            luminance: 15.0,
            luminance_detail: 50.0,
            chroma: 25.0,
            chroma_detail: 40.0,
        },
    };

    let retouch = RetouchParams {
        enabled: true,
        strokes: vec![
            RetouchStroke {
                mode: RetouchMode::Heal,
                radius: 0.01,
                feather: 0.5,
                opacity: 1.0,
                dest: vec![[0.4, 0.6]],
                source_offset: [0.05, 0.0],
            },
            RetouchStroke {
                mode: RetouchMode::Clone,
                radius: 0.02,
                feather: 0.25,
                opacity: 0.75,
                dest: vec![[0.7, 0.3], [0.72, 0.32], [0.74, 0.33]],
                source_offset: [-0.1, 0.05],
            },
        ],
    };

    let geometry = GeometryParams {
        enabled: true,
        crop: Some(CropRect {
            x0: 0.0625,
            y0: 0.125,
            x1: 0.9375,
            y1: 0.875,
        }),
        rotate: -1.5,
        perspective: PerspectiveParams {
            vertical: 10.0,
            horizontal: -5.0,
        },
        flip_horizontal: true,
    };

    let finishing = FinishingParams {
        enabled: true,
        vignette: VignetteParams {
            amount: -30.0,
            midpoint: 40.0,
            roundness: 25.0,
            feather: 60.0,
        },
        grain: GrainParams {
            amount: 20.0,
            size: 30.0,
            roughness: 55.0,
            seed: 0xF0CA_1E00_0000_0001,
        },
    };

    let edit = EditState {
        optics,
        white_balance,
        tone,
        color,
        local,
        detail,
        retouch,
        geometry,
        finishing,
    };

    let live_index = LiveIndex {
        rating: 4,
        flag: Flag::Pick,
        label: Some("Blue".into()),
        capture_time: Some("2026-07-11T09:41:07Z".into()),
        thumbnail_hash: Some(std::array::from_fn(|i| i as u8)),
    };

    let export_recipes = vec![
        ExportRecipe {
            name: "Hand-off TIFF".into(),
            format: ExportFormat::Tiff16 {
                compression: TiffCompression::Deflate,
            },
            color: ExportColor {
                gamut: ExportGamut::AdobeRgb,
            },
            hdr: None,
            resize: None,
        },
        ExportRecipe {
            name: "PNG 16".into(),
            format: ExportFormat::Png { bit_depth: 16 },
            color: ExportColor {
                gamut: ExportGamut::DisplayP3,
            },
            hdr: None,
            resize: Some(ResizeSpec { long_edge: 4096 }),
        },
        ExportRecipe {
            name: "Web JPEG".into(),
            format: ExportFormat::Jpeg { quality: 88 },
            color: ExportColor {
                gamut: ExportGamut::Srgb,
            },
            hdr: None,
            resize: Some(ResizeSpec { long_edge: 2048 }),
        },
        ExportRecipe {
            name: "JXL HDR".into(),
            format: ExportFormat::JpegXl {
                distance: 1.0,
                bit_depth: 16,
            },
            color: ExportColor {
                gamut: ExportGamut::Rec2020,
            },
            hdr: Some(HdrOptions {
                transfer: HdrTransfer::Pq,
                peak_nits: 1000.0,
                gain_map: Some(GainMapOptions {}),
            }),
            resize: None,
        },
        ExportRecipe {
            name: "AVIF HLG".into(),
            format: ExportFormat::Avif {
                quality: 85,
                bit_depth: 10,
            },
            color: ExportColor {
                gamut: ExportGamut::Rec2020,
            },
            hdr: Some(HdrOptions {
                transfer: HdrTransfer::Hlg,
                peak_nits: 1000.0,
                gain_map: None,
            }),
            resize: Some(ResizeSpec { long_edge: 3840 }),
        },
    ];

    SidecarDoc {
        schema_version: SCHEMA_VERSION,
        pipeline_version: focale_core::PIPELINE_VERSION,
        edit,
        live_index,
        export_recipes,
        // Frozen literals, never derived from the running build — the
        // fixture must be identical on every machine.
        focale_version: Some("0.1.0+e258182".into()),
        focale_platform: Some("linux".into()),
    }
}

/// Recursively reverses the entry order of every map in a [`Value`] tree.
/// Deterministic encoding must produce identical bytes regardless of the
/// order entries were built in.
fn reverse_maps(value: Value) -> Value {
    match value {
        Value::Map(entries) => Value::Map(
            entries
                .into_iter()
                .rev()
                .map(|(k, v)| (reverse_maps(k), reverse_maps(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(reverse_maps).collect()),
        Value::Tag(t, inner) => Value::Tag(t, Box::new(reverse_maps(*inner))),
        other => other,
    }
}

#[test]
fn encoding_is_deterministic_and_order_independent() {
    let doc = canonical_doc();
    let b1 = doc.save_to_bytes().unwrap();
    let b2 = doc.save_to_bytes().unwrap();
    assert_eq!(b1, b2, "same document must serialize to identical bytes");

    // A structurally equal tree whose map entries are built in the reverse
    // order must still encode to the same bytes (bytewise key sorting).
    let tree = Value::serialized(&doc).unwrap();
    let reversed = reverse_maps(tree.clone());
    assert_ne!(
        tree, reversed,
        "sanity: the permuted tree differs in entry order"
    );
    let mut fwd = Vec::new();
    cde::write_value(&mut fwd, &Value::Tag(55799, Box::new(tree))).unwrap();
    let mut rev = Vec::new();
    cde::write_value(&mut rev, &Value::Tag(55799, Box::new(reversed))).unwrap();
    assert_eq!(fwd, rev, "map entry order must not affect encoded bytes");
    assert_eq!(fwd, b1, "Value-level encoding must match save_to_bytes");
}

#[test]
fn bytes_match_committed_golden_file() {
    // Golden-byte contract: the canonical document's encoding is frozen.
    // Regenerate deliberately with FOCALE_BLESS=1 (see module docs).
    let golden_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/canonical.fcl"
    ));
    let bytes = canonical_doc().save_to_bytes().unwrap();
    if std::env::var_os("FOCALE_BLESS").is_some() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(golden_path, &bytes).unwrap();
        return;
    }
    let golden = std::fs::read(golden_path).unwrap_or_else(|e| {
        panic!("missing golden file {golden_path:?} ({e}); run with FOCALE_BLESS=1 to create it")
    });
    assert_eq!(
        bytes, golden,
        "sidecar bytes diverged from the committed golden file; if this \
         change is intentional it is a schema change — bump SCHEMA_VERSION \
         and re-bless with FOCALE_BLESS=1"
    );
}

#[test]
fn round_trip_preserves_document() {
    let doc = canonical_doc();
    let bytes = doc.save_to_bytes().unwrap();
    let back = SidecarDoc::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);

    // Untagged documents (bare CBOR, no 55799 envelope) load identically.
    let untagged = cde::to_deterministic_bytes(&doc).unwrap();
    assert_ne!(untagged, bytes);
    let back = SidecarDoc::load_from_bytes(&untagged).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn future_schema_version_is_rejected() {
    let mut doc = canonical_doc();
    doc.schema_version = SCHEMA_VERSION + 1;
    let bytes = doc.save_to_bytes().unwrap();
    match SidecarDoc::load_from_bytes(&bytes) {
        Err(SidecarError::FutureSchema(v)) => assert_eq!(v, SCHEMA_VERSION + 1),
        other => panic!("expected FutureSchema, got {other:?}"),
    }
}

#[test]
fn unknown_map_keys_are_ignored() {
    // Forward tolerance: a future minor schema revision may add keys that
    // this build does not know; loading must ignore them.
    let doc = canonical_doc();
    let tree = Value::serialized(&doc).unwrap();
    let Value::Map(mut entries) = tree else {
        panic!("document must encode as a map");
    };
    entries.push((
        Value::Text("zz_future_field".into()),
        Value::Text("ignored".into()),
    ));
    // Also inject an unknown key into a nested map (the edit state).
    for (k, v) in &mut entries {
        if *k == Value::Text("edit".into()) {
            let Value::Map(edit_entries) = v else {
                panic!("edit must encode as a map");
            };
            edit_entries.push((
                Value::Text("zz_future_stage".into()),
                Value::Integer(7.into()),
            ));
        }
    }
    let mut bytes = Vec::new();
    cde::write_value(
        &mut bytes,
        &Value::Tag(55799, Box::new(Value::Map(entries))),
    )
    .unwrap();
    let back = SidecarDoc::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn fs_save_and_load_round_trip() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-fs");
    std::fs::create_dir_all(&dir).unwrap();
    let image_path = dir.join("IMG_0001.ARW");
    let sidecar_path = focale_sidecar::sidecar_path_for(&image_path);
    assert_eq!(sidecar_path, dir.join("IMG_0001.ARW.fcl"));

    let doc = canonical_doc();
    doc.save(&sidecar_path).unwrap();
    let back = SidecarDoc::load(&sidecar_path).unwrap();
    assert_eq!(back, doc);

    // Overwriting atomically leaves the new content in place.
    let mut doc2 = doc.clone();
    doc2.live_index.rating = 5;
    doc2.save(&sidecar_path).unwrap();
    assert_eq!(SidecarDoc::load(&sidecar_path).unwrap(), doc2);
}
