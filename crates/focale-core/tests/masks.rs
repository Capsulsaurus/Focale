//! Integration tests for the pipeline v1 mask rasterizer
//! (`pipeline::v1::masks`) — the architecture.md §6 parity checklist backbone.
//!
//! Geometry probes use normalized coordinates and the pixel-centre
//! convention: pixel `(x, y)` samples `((x + 0.5)/w, (y + 0.5)/h)`.

use focale_core::image::{ImageGrayF32, ImageRgbF32};
use focale_core::masks::{
    BrushMask, BrushStroke, ColorRangeMask, LinearGradientMask, LuminanceRangeMask, MaskComponent,
    MaskGroup, MaskOp, MaskShape, RadialGradientMask, ResolvedMask, SegmentKind,
};
use focale_core::pipeline::v1::masks::{MaskContext, rasterize_component, rasterize_group};

// ---------------------------------------------------------------- helpers

fn ctx<'a>(image: &'a ImageRgbF32) -> MaskContext<'a> {
    MaskContext {
        width: image.width(),
        height: image.height(),
        image,
    }
}

/// A component with neutral settings: Add, no invert, no feather, density 1.
fn comp(shape: MaskShape) -> MaskComponent {
    MaskComponent {
        op: MaskOp::Add,
        invert: false,
        feather: 0.0,
        density: 1.0,
        shape,
    }
}

fn comp_op(op: MaskOp, shape: MaskShape) -> MaskComponent {
    MaskComponent { op, ..comp(shape) }
}

fn group(components: Vec<MaskComponent>) -> MaskGroup {
    MaskGroup {
        name: "test".to_string(),
        components,
    }
}

/// Rasterizes a bare shape on a black image of the given size.
fn raster(shape: MaskShape, width: u32, height: u32) -> ImageGrayF32 {
    let image = ImageRgbF32::new(width, height);
    rasterize_component(&comp(shape), &ctx(&image))
}

/// Reads the plane at the pixel nearest to normalized coordinates.
fn probe(m: &ImageGrayF32, nx: f32, ny: f32) -> f32 {
    let x = (nx * m.width() as f32 - 0.5)
        .round()
        .clamp(0.0, (m.width() - 1) as f32) as u32;
    let y = (ny * m.height() as f32 - 0.5)
        .round()
        .clamp(0.0, (m.height() - 1) as f32) as u32;
    m.get(x, y)
}

fn assert_close(actual: f32, expected: f32, tol: f32) {
    assert!(
        (actual - expected).abs() <= tol,
        "{actual} differs from {expected} by more than {tol}"
    );
}

/// A hard-edged circle (radial gradient, falloff 0).
fn disk(cx: f32, cy: f32, r: f32) -> MaskShape {
    MaskShape::Radial(RadialGradientMask {
        center: [cx, cy],
        radius: [r, r],
        rotation: 0.0,
        falloff: 0.0,
    })
}

/// Wraps bytes in a raw-DEFLATE stored (uncompressed) block, RFC 1951 §3.2.4:
/// `BFINAL=1, BTYPE=00`, then LEN/NLEN little-endian, then the data.
fn stored_deflate(data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= u16::MAX as usize);
    let len = data.len() as u16;
    let mut out = vec![0x01];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out
}

// ---------------------------------------------------------------- linear

#[test]
fn linear_gradient_is_a_straight_ramp() {
    // w=4: pixel centres at nx = 0.125, 0.375, 0.625, 0.875.
    let m = raster(
        MaskShape::Linear(LinearGradientMask {
            start: [0.375, 0.5],
            end: [0.875, 0.5],
        }),
        4,
        1,
    );
    assert_close(m.get(0, 0), 1.0, 1e-6); // t = -0.5, clamped to full
    assert_close(m.get(1, 0), 1.0, 1e-6); // t = 0
    assert_close(m.get(2, 0), 0.5, 1e-6); // t = 0.5
    assert_close(m.get(3, 0), 0.0, 1e-6); // t = 1
}

#[test]
fn linear_gradient_degenerate_is_empty() {
    let m = raster(
        MaskShape::Linear(LinearGradientMask {
            start: [0.5, 0.5],
            end: [0.5, 0.5],
        }),
        8,
        8,
    );
    assert!(m.data().iter().all(|&v| v == 0.0));
}

// ---------------------------------------------------------------- radial

#[test]
fn radial_hard_disk_is_one_inside_zero_outside() {
    let m = raster(disk(0.5, 0.5, 0.25), 64, 64);
    assert_eq!(probe(&m, 0.5, 0.5), 1.0);
    assert_eq!(probe(&m, 0.7, 0.5), 1.0); // e < 1
    assert_eq!(probe(&m, 0.8, 0.5), 0.0); // e > 1
    assert_eq!(probe(&m, 0.05, 0.05), 0.0);
}

#[test]
fn radial_falloff_eases_monotonically() {
    let m = raster(
        MaskShape::Radial(RadialGradientMask {
            center: [0.5, 0.5],
            radius: [0.4, 0.4],
            rotation: 0.0,
            falloff: 1.0,
        }),
        64,
        64,
    );
    assert!(probe(&m, 0.5, 0.5) > 0.99);
    for x in 32..63 {
        assert!(
            m.get(x + 1, 32) <= m.get(x, 32) + 1e-6,
            "radial falloff not monotonic at x={x}"
        );
    }
    assert_eq!(m.get(63, 32), 0.0); // e > 1 at the border
}

#[test]
fn radial_rotation_swings_the_ellipse_axes() {
    let ellipse = |rotation: f32| {
        raster(
            MaskShape::Radial(RadialGradientMask {
                center: [0.5, 0.5],
                radius: [0.4, 0.1],
                rotation,
                falloff: 0.0,
            }),
            64,
            64,
        )
    };
    let flat = ellipse(0.0);
    let rotated = ellipse(90.0);
    // A point 0.3 below the centre is outside the flat ellipse (0.3/0.1 = 3)
    // but inside the 90°-rotated one (0.3/0.4 < 1).
    assert_eq!(probe(&flat, 0.5, 0.8), 0.0);
    assert_eq!(probe(&rotated, 0.5, 0.8), 1.0);
    // And vice versa along x.
    assert_eq!(probe(&flat, 0.8, 0.5), 1.0);
    assert_eq!(probe(&rotated, 0.8, 0.5), 0.0);
}

#[test]
fn radial_zero_radius_is_empty() {
    let m = raster(
        MaskShape::Radial(RadialGradientMask {
            center: [0.5, 0.5],
            radius: [0.0, 0.25],
            rotation: 0.0,
            falloff: 0.5,
        }),
        16,
        16,
    );
    assert!(m.data().iter().all(|&v| v == 0.0));
}

// ---------------------------------------------------------------- brush

/// Normalized coordinates of the centre of pixel (16,16) on a 32×32 plane.
const P16: [f32; 2] = [16.5 / 32.0, 16.5 / 32.0];

fn stroke(erase: bool, flow: f32, feather: f32, points: Vec<[f32; 2]>) -> BrushStroke {
    BrushStroke {
        erase,
        // 0.25 of the 32 px long edge = 8 px stamp radius.
        radius: 0.25,
        feather,
        flow,
        points,
    }
}

fn brush(strokes: Vec<BrushStroke>) -> MaskShape {
    MaskShape::Brush(BrushMask { strokes })
}

#[test]
fn brush_single_stamp_profile() {
    // One stamp at the centre of pixel (16,16): radius 8 px, feather 0.5
    // → flat inside 4 px, smoothstep to 0 at 8 px.
    let m = raster(brush(vec![stroke(false, 1.0, 0.5, vec![P16])]), 32, 32);
    assert_eq!(m.get(16, 16), 1.0); // d = 0
    assert_eq!(m.get(18, 16), 1.0); // d = 2 <= inner
    assert_close(m.get(22, 16), 0.5, 1e-6); // d = 6: midpoint of the falloff
    assert_eq!(m.get(26, 16), 0.0); // d = 10 > radius
    assert_eq!(m.get(16, 26), 0.0);
}

#[test]
fn brush_flow_builds_up_across_strokes() {
    let one = raster(brush(vec![stroke(false, 0.5, 0.0, vec![P16])]), 32, 32);
    assert_close(one.get(16, 16), 0.5, 1e-6);

    let two = raster(
        brush(vec![
            stroke(false, 0.5, 0.0, vec![P16]),
            stroke(false, 0.5, 0.0, vec![P16]),
        ]),
        32,
        32,
    );
    // 0.5 + 0.5·(1 − 0.5) = 0.75.
    assert_close(two.get(16, 16), 0.75, 1e-6);
}

#[test]
fn brush_eraser_subtracts_from_painted_mask() {
    let erased = raster(
        brush(vec![
            stroke(false, 1.0, 0.0, vec![P16]),
            stroke(true, 0.25, 0.0, vec![P16]),
        ]),
        32,
        32,
    );
    // 1 · (1 − 0.25) = 0.75.
    assert_close(erased.get(16, 16), 0.75, 1e-6);

    let fully = raster(
        brush(vec![
            stroke(false, 1.0, 0.0, vec![P16]),
            stroke(true, 1.0, 0.0, vec![P16]),
        ]),
        32,
        32,
    );
    assert_eq!(fully.get(16, 16), 0.0);

    // An eraser on an empty mask stays empty.
    let empty = raster(brush(vec![stroke(true, 1.0, 0.0, vec![P16])]), 32, 32);
    assert!(empty.data().iter().all(|&v| v == 0.0));
}

#[test]
fn brush_stroke_marches_the_polyline() {
    // Horizontal drag across the plane at the row of pixel centres y=16.
    let m = raster(
        brush(vec![stroke(
            false,
            1.0,
            0.0,
            vec![[0.1, P16[1]], [0.9, P16[1]]],
        )]),
        32,
        32,
    );
    // Stamp spacing (2 px) is far below the 8 px radius, so the swept row is
    // solid, while rows further than the radius from the path stay empty.
    for x in 0..32 {
        assert_eq!(m.get(x, 16), 1.0, "row 16 not solid at x={x}");
        assert_eq!(m.get(x, 0), 0.0, "row 0 painted at x={x}");
        assert_eq!(m.get(x, 31), 0.0, "row 31 painted at x={x}");
    }
}

#[test]
fn brush_empty_stroke_paints_nothing() {
    let m = raster(brush(vec![stroke(false, 1.0, 0.0, vec![])]), 16, 16);
    assert!(m.data().iter().all(|&v| v == 0.0));
}

// ---------------------------------------------------------------- ranges

#[test]
fn luminance_range_selects_the_bright_half() {
    // Gray gradient: srgb_encode(0.05) ≈ 0.245, srgb_encode(0.5) ≈ 0.735.
    let mut image = ImageRgbF32::new(2, 1);
    image.set_pixel(0, 0, [0.05, 0.05, 0.05]);
    image.set_pixel(1, 0, [0.5, 0.5, 0.5]);
    let shape = MaskShape::LuminanceRange(LuminanceRangeMask {
        low: 0.5,
        high: 1.0,
        falloff: 0.0,
    });
    let m = rasterize_component(&comp(shape), &ctx(&image));
    assert_eq!(m.get(0, 0), 0.0);
    assert_eq!(m.get(1, 0), 1.0);
}

#[test]
fn luminance_range_falloff_is_partial_at_the_edge() {
    let mut image = ImageRgbF32::new(1, 1);
    image.set_pixel(0, 0, [0.5, 0.5, 0.5]); // encoded ≈ 0.735
    let shape = MaskShape::LuminanceRange(LuminanceRangeMask {
        low: 0.75,
        high: 1.0,
        falloff: 0.5, // ramp half-width 0.25: rising edge spans [0.5, 0.75]
    });
    let m = rasterize_component(&comp(shape), &ctx(&image));
    let v = m.get(0, 0);
    assert!(0.9 < v && v < 1.0, "expected partial coverage, got {v}");
}

#[test]
fn color_range_selects_red_not_blue() {
    let red = [0.6, 0.05, 0.05];
    let blue = [0.05, 0.05, 0.6];
    let mut image = ImageRgbF32::new(2, 1);
    image.set_pixel(0, 0, red);
    image.set_pixel(1, 0, blue);
    let shape = MaskShape::ColorRange(ColorRangeMask {
        color: red,
        tolerance: 0.2,
        falloff: 0.5,
    });
    let m = rasterize_component(&comp(shape), &ctx(&image));
    assert_eq!(m.get(0, 0), 1.0); // d = 0
    assert_eq!(m.get(1, 0), 0.0); // Oklab red↔blue distance ≫ 0.2
}

#[test]
fn color_range_zero_tolerance_selects_nothing() {
    let mut image = ImageRgbF32::new(1, 1);
    image.set_pixel(0, 0, [0.3, 0.3, 0.3]);
    let shape = MaskShape::ColorRange(ColorRangeMask {
        color: [0.3, 0.3, 0.3],
        tolerance: 0.0,
        falloff: 0.5,
    });
    let m = rasterize_component(&comp(shape), &ctx(&image));
    assert_eq!(m.get(0, 0), 0.0);
}

// ---------------------------------------------------------------- AI mask

fn resolved(width: u32, height: u32, bitmap: &[u8]) -> MaskShape {
    MaskShape::AiResolved(ResolvedMask {
        kind: SegmentKind::Sky,
        width,
        height,
        deflate_bitmap: stored_deflate(bitmap),
    })
}

#[test]
fn ai_resolved_roundtrips_at_native_size() {
    let m = raster(resolved(2, 2, &[0, 255, 255, 0]), 2, 2);
    assert_eq!(m.data(), &[0.0, 1.0, 1.0, 0.0]);
}

#[test]
fn ai_resolved_upsamples_bilinearly() {
    let bitmap = [0u8, 255, 255, 0];
    let m = raster(resolved(2, 2, &bitmap), 4, 4);
    // Expected: bilinear samples of the low-res plane under the
    // pixel-centre mapping src = (dst + 0.5)·src/dst − 0.5.
    let src = ImageGrayF32::from_data(2, 2, bitmap.iter().map(|&b| b as f32 / 255.0).collect());
    for y in 0..4 {
        for x in 0..4 {
            let expected =
                src.sample_bilinear((x as f32 + 0.5) * 0.5 - 0.5, (y as f32 + 0.5) * 0.5 - 0.5);
            assert_eq!(m.get(x, y), expected, "mismatch at ({x},{y})");
        }
    }
    // Interior of the upsample is a genuine blend.
    let centre = m.get(1, 1);
    assert!(0.0 < centre && centre < 1.0);
}

#[test]
fn ai_resolved_bad_payload_is_empty() {
    // Valid stream, wrong decompressed length for the declared dimensions.
    let m = raster(resolved(2, 2, &[1, 2, 3]), 4, 4);
    assert!(m.data().iter().all(|&v| v == 0.0));

    // Garbage stream.
    let shape = MaskShape::AiResolved(ResolvedMask {
        kind: SegmentKind::Subject,
        width: 2,
        height: 2,
        deflate_bitmap: vec![0xde, 0xad, 0xbe, 0xef],
    });
    let m = raster(shape, 4, 4);
    assert!(m.data().iter().all(|&v| v == 0.0));
}

// ------------------------------------------------------- ops & pipeline

// Two hard disks overlapping in the middle of a 64×64 plane. Probe points:
// PA in A only, PM in both, PB in B only, PO in neither.
fn disk_a() -> MaskShape {
    disk(0.375, 0.5, 0.25)
}
fn disk_b() -> MaskShape {
    disk(0.625, 0.5, 0.25)
}
const PA: [f32; 2] = [0.25, 0.5];
const PM: [f32; 2] = [0.5, 0.5];
const PB: [f32; 2] = [0.75, 0.5];
const PO: [f32; 2] = [0.05, 0.05];

fn raster_group(components: Vec<MaskComponent>) -> ImageGrayF32 {
    let image = ImageRgbF32::new(64, 64);
    rasterize_group(&group(components), &ctx(&image))
}

fn probes(m: &ImageGrayF32) -> [f32; 4] {
    [
        probe(m, PA[0], PA[1]),
        probe(m, PM[0], PM[1]),
        probe(m, PB[0], PB[1]),
        probe(m, PO[0], PO[1]),
    ]
}

#[test]
fn add_is_union() {
    let m = raster_group(vec![comp(disk_a()), comp(disk_b())]);
    assert_eq!(probes(&m), [1.0, 1.0, 1.0, 0.0]);
}

#[test]
fn subtract_removes_coverage() {
    let m = raster_group(vec![comp(disk_a()), comp_op(MaskOp::Subtract, disk_b())]);
    assert_eq!(probes(&m), [1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn intersect_keeps_the_overlap() {
    let m = raster_group(vec![comp(disk_a()), comp_op(MaskOp::Intersect, disk_b())]);
    assert_eq!(probes(&m), [0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn invert_flips_a_component() {
    let inverted = MaskComponent {
        invert: true,
        ..comp(disk_a())
    };
    let m = raster_group(vec![inverted]);
    assert_eq!(probes(&m), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn component_order_matters() {
    // Subtract after Add clears the overlap; Subtract first is a no-op on
    // empty coverage, so the later Add restores it.
    let sub_after_add = raster_group(vec![comp(disk_a()), comp_op(MaskOp::Subtract, disk_b())]);
    let add_after_sub = raster_group(vec![comp_op(MaskOp::Subtract, disk_b()), comp(disk_a())]);
    assert_eq!(probe(&sub_after_add, PM[0], PM[1]), 0.0);
    assert_eq!(probe(&add_after_sub, PM[0], PM[1]), 1.0);
    assert_eq!(probes(&add_after_sub), [1.0, 1.0, 0.0, 0.0]);
}

#[test]
fn empty_group_is_all_zero() {
    let m = raster_group(vec![]);
    assert!(m.data().iter().all(|&v| v == 0.0));
}

#[test]
fn leading_subtract_or_intersect_stays_zero() {
    let m = raster_group(vec![comp_op(MaskOp::Subtract, disk_a())]);
    assert!(m.data().iter().all(|&v| v == 0.0));
    let m = raster_group(vec![comp_op(MaskOp::Intersect, disk_a())]);
    assert!(m.data().iter().all(|&v| v == 0.0));
}

// ------------------------------------------------------ feather & density

#[test]
fn feather_blurs_the_edge_monotonically() {
    let image = ImageRgbF32::new(64, 64);
    let feathered = MaskComponent {
        feather: 0.1, // sigma = 0.1·64/3 ≈ 2.13 px, kernel radius 7 px
        ..comp(disk(0.5, 0.5, 0.25))
    };
    let m = rasterize_component(&feathered, &ctx(&image));
    // Centre still saturated, corner untouched by the finite kernel.
    assert!(m.get(32, 32) > 0.99);
    assert_eq!(m.get(0, 0), 0.0);
    // The blurred boundary (16 px from centre) is a genuine intermediate.
    let edge = m.get(48, 32);
    assert!(0.1 < edge && edge < 0.9, "edge value {edge}");
    // Monotonic non-increasing from the centre outward along the row.
    for x in 32..63 {
        assert!(
            m.get(x + 1, 32) <= m.get(x, 32) + 1e-6,
            "feathered edge not monotonic at x={x}"
        );
    }
    // Coverage stays in [0, 1] (normalized kernel).
    assert!(m.data().iter().all(|&v| (0.0..=1.0 + 1e-6).contains(&v)));
}

#[test]
fn density_scales_the_maximum() {
    let image = ImageRgbF32::new(64, 64);
    let half = MaskComponent {
        density: 0.5,
        ..comp(disk(0.5, 0.5, 0.25))
    };
    let m = rasterize_component(&half, &ctx(&image));
    let max = m.data().iter().fold(0.0f32, |a, &v| a.max(v));
    assert_eq!(max, 0.5);
    assert_eq!(probe(&m, 0.5, 0.5), 0.5);
    assert_eq!(probe(&m, 0.05, 0.05), 0.0);
}

// ----------------------------------------------------------- determinism

/// A group exercising every shape type plus invert, feather and density.
fn kitchen_sink() -> MaskGroup {
    group(vec![
        comp(brush(vec![stroke(
            false,
            0.8,
            0.7,
            vec![[0.2, 0.3], [0.6, 0.4], [0.7, 0.8]],
        )])),
        MaskComponent {
            op: MaskOp::Add,
            invert: true,
            feather: 0.05,
            density: 0.9,
            shape: MaskShape::Linear(LinearGradientMask {
                start: [0.1, 0.2],
                end: [0.8, 0.9],
            }),
        },
        comp_op(
            MaskOp::Intersect,
            MaskShape::Radial(RadialGradientMask {
                center: [0.4, 0.6],
                radius: [0.5, 0.3],
                rotation: 30.0,
                falloff: 0.6,
            }),
        ),
        comp_op(
            MaskOp::Subtract,
            MaskShape::LuminanceRange(LuminanceRangeMask {
                low: 0.2,
                high: 0.7,
                falloff: 0.3,
            }),
        ),
        comp(MaskShape::ColorRange(ColorRangeMask {
            color: [0.4, 0.2, 0.1],
            tolerance: 0.3,
            falloff: 0.4,
        })),
        comp_op(
            MaskOp::Intersect,
            resolved(3, 2, &[10, 200, 40, 250, 0, 128]),
        ),
    ])
}

#[test]
fn rasterization_is_bit_identical_across_runs() {
    // Deterministic non-trivial working image.
    let (w, h) = (48u32, 32u32);
    let mut image = ImageRgbF32::new(w, h);
    for y in 0..h {
        for x in 0..w {
            image.set_pixel(
                x,
                y,
                [
                    x as f32 / w as f32,
                    y as f32 / h as f32,
                    ((x + 2 * y) % 7) as f32 / 7.0,
                ],
            );
        }
    }
    let g = kitchen_sink();
    let a = rasterize_group(&g, &ctx(&image));
    let b = rasterize_group(&g, &ctx(&image));
    assert_eq!(a.data().len(), b.data().len());
    for (i, (va, vb)) in a.data().iter().zip(b.data()).enumerate() {
        assert_eq!(va.to_bits(), vb.to_bits(), "bit mismatch at sample {i}");
    }
    // And the output is a valid coverage plane.
    assert!(a.data().iter().all(|&v| (0.0..=1.0).contains(&v)));
}
