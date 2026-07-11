//! Real-inference integration tests, `#[ignore]`d by default.
//!
//! These require the ONNX models on disk and are therefore not part of
//! `cargo test` (which must pass without models or network). To run them:
//!
//! ```sh
//! scripts/fetch-models.sh                    # once, downloads the models
//! cargo test -p focale-segment -- --ignored
//! ```
//!
//! The models directory is `$FOCALE_MODELS_DIR` if set, otherwise the
//! standard user location (`$XDG_DATA_HOME/focale/models` or
//! `~/.local/share/focale/models`). Each test panics with instructions if
//! its model is missing — a skipped assertion would hide breakage.

use flate2::read::DeflateDecoder;
use focale_core::image::ImageRgbF32;
use focale_core::masks::{PersonPart, ResolvedMask, SegmentKind};
use focale_segment::{ModelKind, ModelPaths, Segmenter};
use std::io::Read;

fn model_paths() -> ModelPaths {
    match std::env::var_os("FOCALE_MODELS_DIR") {
        Some(dir) => ModelPaths::new(dir),
        None => ModelPaths::user_default(),
    }
}

fn segmenter_with(kinds: &[ModelKind]) -> Segmenter {
    let paths = model_paths();
    for &kind in kinds {
        assert!(
            paths.available(kind),
            "model {kind:?} not installed in {} — run scripts/fetch-models.sh \
             or set FOCALE_MODELS_DIR",
            paths.root().display()
        );
    }
    Segmenter::new(paths)
}

/// A 96×64 synthetic scene: dark ground, bright bluish top half ("sky"),
/// bright disc left of centre ("subject").
fn synthetic_image() -> ImageRgbF32 {
    let (w, h) = (96u32, 64u32);
    let mut img = ImageRgbF32::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut px = if y < h / 2 {
                [0.35, 0.55, 0.95] // sky-ish
            } else {
                [0.05, 0.06, 0.05] // ground
            };
            let (dx, dy) = (x as f32 - 32.0, y as f32 - 40.0);
            if dx * dx + dy * dy < 14.0 * 14.0 {
                px = [0.9, 0.8, 0.2]; // subject disc
            }
            img.set_pixel(x, y, px);
        }
    }
    img
}

/// Inflates the mask exactly as `focale_core::pipeline::v1` does and sanity
/// checks the dimensions against half the working resolution.
fn inflate_and_check(mask: &ResolvedMask, image: &ImageRgbF32) -> Vec<u8> {
    assert_eq!(mask.width, image.width().div_ceil(2));
    assert_eq!(mask.height, image.height().div_ceil(2));
    let mut bytes = Vec::new();
    DeflateDecoder::new(mask.deflate_bitmap.as_slice())
        .read_to_end(&mut bytes)
        .expect("raw DEFLATE stream");
    assert_eq!(bytes.len(), mask.width as usize * mask.height as usize);
    bytes
}

#[test]
#[ignore = "requires downloaded models (see module docs)"]
fn subject_and_background_are_complementary() {
    let mut seg = segmenter_with(&[ModelKind::Saliency]);
    let img = synthetic_image();

    let subject = seg.subject(&img).expect("subject inference");
    assert_eq!(subject.kind, SegmentKind::Subject);
    let subject_bytes = inflate_and_check(&subject, &img);
    assert!(subject_bytes.iter().any(|&b| b > 128), "some coverage");
    assert!(subject_bytes.iter().any(|&b| b < 128), "not all coverage");

    let background = seg.background(&img).expect("background inference");
    assert_eq!(background.kind, SegmentKind::Background);
    let background_bytes = inflate_and_check(&background, &img);
    // Complementary up to quantization.
    for (s, b) in subject_bytes.iter().zip(&background_bytes) {
        assert!(
            (i16::from(*s) + i16::from(*b) - 255).abs() <= 1,
            "{s} + {b}"
        );
    }
}

#[test]
#[ignore = "requires downloaded models (see module docs)"]
fn sky_mask_prefers_the_upper_half() {
    let mut seg = segmenter_with(&[ModelKind::Sky]);
    let img = synthetic_image();
    let mask = seg.sky(&img).expect("sky inference");
    assert_eq!(mask.kind, SegmentKind::Sky);
    let bytes = inflate_and_check(&mask, &img);
    let w = mask.width as usize;
    let h = mask.height as usize;
    let mean = |rows: std::ops::Range<usize>| -> f64 {
        let mut sum = 0.0;
        for y in rows.clone() {
            for x in 0..w {
                sum += f64::from(bytes[y * w + x]);
            }
        }
        sum / (rows.len() * w) as f64
    };
    let top = mean(0..h / 2);
    let bottom = mean(h / 2..h);
    assert!(
        top > bottom,
        "sky coverage should concentrate in the top half (top {top:.1}, bottom {bottom:.1})"
    );
}

#[test]
#[ignore = "requires downloaded models (see module docs)"]
fn object_click_selects_around_the_point() {
    let mut seg = segmenter_with(&[ModelKind::SamEncoder, ModelKind::SamDecoder]);
    let img = synthetic_image();
    // Click the centre of the bright disc at (32, 40) in a 96×64 frame.
    let mask = seg
        .object_at(&img, [32.0 / 96.0, 40.0 / 64.0])
        .expect("SAM inference");
    assert_eq!(mask.kind, SegmentKind::Object);
    let bytes = inflate_and_check(&mask, &img);
    let w = mask.width as usize;
    // The clicked pixel (half-res 16, 20) must be selected.
    assert!(bytes[20 * w + 16] > 128, "clicked point covered");
    // A far corner must not be.
    assert!(bytes[w - 1] < 128, "far corner not covered");

    // A second click reuses the cached embedding (observable only as a much
    // faster call; correctness-wise it must still produce a valid mask).
    let again = seg
        .object_at(&img, [32.0 / 96.0, 40.0 / 64.0])
        .expect("SAM inference (cached embedding)");
    assert_eq!(inflate_and_check(&again, &img), bytes);
}

#[test]
#[ignore = "requires downloaded models (see module docs)"]
fn person_and_parts_produce_valid_masks() {
    let mut seg = segmenter_with(&[ModelKind::FaceParsing]);
    let img = synthetic_image();
    // No face in the synthetic scene: assert shape/range validity, not
    // content.
    let person = seg.person(&img).expect("person inference");
    assert_eq!(person.kind, SegmentKind::Person { index: 0 });
    inflate_and_check(&person, &img);

    let part = seg
        .person_part(&img, PersonPart::Hair)
        .expect("person part inference");
    assert_eq!(
        part.kind,
        SegmentKind::PersonPart {
            index: 0,
            part: PersonPart::Hair
        }
    );
    inflate_and_check(&part, &img);
}
