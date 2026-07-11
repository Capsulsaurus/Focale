//! Transfer functions (opto-electronic encodings).
//!
//! Conventions: `encode` maps linear light to a non-linear signal
//! (OETF / inverse EOTF), `decode` maps a signal back to linear light. All
//! functions are `f32`, clamp their input to the valid domain, and follow
//! the defining specification exactly:
//!
//! - sRGB piecewise curve: IEC 61966-2-1. Display P3 uses this same
//!   transfer (its encoding is DCI-P3 primaries + D65 + the sRGB curve).
//! - Adobe RGB (1998): pure gamma of exactly 563/256 (Adobe RGB (1998)
//!   Color Image Encoding, §4.3.1.2).
//! - PQ: SMPTE ST 2084 / ITU-R BT.2100-2, with SDR reference-white helpers
//!   per ITU-R BT.2408 (SDR diffuse white = 203 cd/m²).
//! - HLG: ARIB STD-B67 / ITU-R BT.2100-2 scene-referred OETF.
//!
//! `powf`/`ln`/`exp` come from the platform maths library; see the module
//! documentation in [`crate::color`] for the determinism caveat.

/// Linear light [0, 1] → sRGB signal [0, 1] (IEC 61966-2-1). Input clamped.
pub fn srgb_encode(linear: f32) -> f32 {
    let l = linear.clamp(0.0, 1.0);
    if l <= 0.0031308 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB signal [0, 1] → linear light [0, 1] (IEC 61966-2-1). Input clamped.
pub fn srgb_decode(signal: f32) -> f32 {
    let s = signal.clamp(0.0, 1.0);
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// The Adobe RGB (1998) gamma, exactly 563/256 = 2.19921875.
pub const ADOBE_RGB_GAMMA: f32 = 563.0 / 256.0;

/// Linear light [0, 1] → Adobe RGB (1998) signal [0, 1]. Input clamped.
pub fn adobe_rgb_encode(linear: f32) -> f32 {
    linear.clamp(0.0, 1.0).powf(1.0 / ADOBE_RGB_GAMMA)
}

/// Adobe RGB (1998) signal [0, 1] → linear light [0, 1]. Input clamped.
pub fn adobe_rgb_decode(signal: f32) -> f32 {
    signal.clamp(0.0, 1.0).powf(ADOBE_RGB_GAMMA)
}

// SMPTE ST 2084 constants; every value is exactly representable in f32.
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;

/// Peak luminance of the PQ signal, in cd/m² (SMPTE ST 2084).
pub const PQ_PEAK_LUMINANCE: f32 = 10_000.0;

/// SDR reference white on the PQ scale, in cd/m² (ITU-R BT.2408).
pub const PQ_SDR_WHITE_LUMINANCE: f32 = 203.0;

/// Normalized linear light [0, 1] (1.0 = 10 000 cd/m²) → PQ signal [0, 1]
/// (SMPTE ST 2084 inverse EOTF). Input clamped.
pub fn pq_encode(linear: f32) -> f32 {
    let y = linear.clamp(0.0, 1.0);
    let p = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * p) / (1.0 + PQ_C3 * p)).powf(PQ_M2)
}

/// PQ signal [0, 1] → normalized linear light [0, 1] (1.0 = 10 000 cd/m²)
/// (SMPTE ST 2084 EOTF). Input clamped.
pub fn pq_decode(signal: f32) -> f32 {
    let s = signal.clamp(0.0, 1.0);
    let p = s.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    (num / den).powf(1.0 / PQ_M1)
}

/// SDR-referred linear light → PQ signal, mapping linear 1.0 to SDR
/// reference white (203 cd/m², ITU-R BT.2408). Accepts HDR input up to
/// linear ≈ 49.26 (= 10 000 / 203); anything brighter clamps to signal 1.0.
pub fn pq_encode_sdr(linear: f32) -> f32 {
    pq_encode(linear * (PQ_SDR_WHITE_LUMINANCE / PQ_PEAK_LUMINANCE))
}

/// PQ signal → SDR-referred linear light where 1.0 is SDR reference white
/// (203 cd/m², ITU-R BT.2408). Inverse of [`pq_encode_sdr`].
pub fn pq_decode_sdr(signal: f32) -> f32 {
    pq_decode(signal) * (PQ_PEAK_LUMINANCE / PQ_SDR_WHITE_LUMINANCE)
}

// HLG constants (ARIB STD-B67 / ITU-R BT.2100-2):
// a, b = 1 − 4a, c = 0.5 − a·ln(4a).
const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.5599107;

/// Scene-referred linear light [0, 1] → HLG signal [0, 1]
/// (ARIB STD-B67 / ITU-R BT.2100-2 OETF). Input clamped.
pub fn hlg_oetf(scene: f32) -> f32 {
    let e = scene.clamp(0.0, 1.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
    }
}

/// HLG signal [0, 1] → scene-referred linear light [0, 1]
/// (inverse of [`hlg_oetf`]). Input clamped.
pub fn hlg_oetf_inverse(signal: f32) -> f32 {
    let s = signal.clamp(0.0, 1.0);
    if s <= 0.5 {
        (s * s) / 3.0
    } else {
        (((s - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::testutil::assert_close;

    #[test]
    fn srgb_round_trip() {
        for i in 0..=100 {
            let l = i as f32 / 100.0;
            assert_close(srgb_decode(srgb_encode(l)), l, 1e-5);
            assert_close(srgb_encode(srgb_decode(l)), l, 1e-5);
        }
    }

    #[test]
    fn srgb_known_values() {
        assert_eq!(srgb_encode(0.0), 0.0);
        assert_close(srgb_encode(1.0), 1.0, 1e-6);
        assert_close(srgb_encode(0.5), 0.735357, 1e-4);
        assert_close(srgb_decode(1.0), 1.0, 1e-6);
        assert_close(srgb_decode(0.735357), 0.5, 1e-4);
    }

    #[test]
    fn srgb_clamps_out_of_range() {
        assert_eq!(srgb_encode(-0.5), 0.0);
        assert_eq!(srgb_encode(2.0), srgb_encode(1.0));
        assert_eq!(srgb_decode(-1.0), 0.0);
        assert_eq!(srgb_decode(1.5), srgb_decode(1.0));
    }

    #[test]
    fn srgb_piecewise_is_continuous() {
        let below = srgb_encode(0.0031307);
        let above = srgb_encode(0.0031309);
        assert!((below - above).abs() < 1e-5);
    }

    #[test]
    fn adobe_rgb_round_trip_and_gamma() {
        assert_eq!(ADOBE_RGB_GAMMA, 563.0 / 256.0);
        for i in 0..=100 {
            let l = i as f32 / 100.0;
            assert_close(adobe_rgb_decode(adobe_rgb_encode(l)), l, 1e-5);
        }
        assert_eq!(adobe_rgb_encode(0.0), 0.0);
        assert_eq!(adobe_rgb_encode(1.0), 1.0);
        assert_eq!(adobe_rgb_encode(-1.0), 0.0);
        assert_eq!(adobe_rgb_encode(2.0), 1.0);
    }

    #[test]
    fn pq_known_values() {
        // The ST 2084 formula yields c1^m2 ≈ 7.3e-7 at zero (the EOTF is
        // flat below that signal, so decode(encode(0.0)) is still 0.0).
        assert_close(pq_encode(0.0), 0.0, 1e-6);
        assert_eq!(pq_decode(pq_encode(0.0)), 0.0);
        assert_eq!(pq_decode(0.0), 0.0);
        assert_close(pq_encode(1.0), 1.0, 1e-6);
        // SDR reference white (203 cd/m²) sits at ≈58% PQ (ITU-R BT.2408).
        assert_close(pq_encode_sdr(1.0), 0.5806889, 1e-4);
        assert_close(pq_decode_sdr(0.5806889), 1.0, 1e-3);
    }

    #[test]
    fn pq_round_trip() {
        for y in [0.0_f32, 1e-4, 1e-3, 0.0203, 0.1, 0.5, 1.0] {
            let back = pq_decode(pq_encode(y));
            assert!(
                (back - y).abs() <= 1e-6 + 1e-3 * y,
                "pq round trip at {y}: {back}"
            );
        }
    }

    #[test]
    fn pq_sdr_peak_maps_to_one() {
        assert_close(
            pq_encode_sdr(PQ_PEAK_LUMINANCE / PQ_SDR_WHITE_LUMINANCE),
            1.0,
            1e-5,
        );
        assert_eq!(pq_encode_sdr(100.0), 1.0); // beyond peak clamps
    }

    #[test]
    fn hlg_known_values() {
        assert_eq!(hlg_oetf(0.0), 0.0);
        assert_close(hlg_oetf(1.0 / 12.0), 0.5, 1e-6);
        assert_close(hlg_oetf(1.0), 1.0, 1e-4);
        assert_close(hlg_oetf_inverse(0.5), 1.0 / 12.0, 1e-6);
        assert_close(hlg_oetf_inverse(1.0), 1.0, 1e-4);
    }

    #[test]
    fn hlg_round_trip() {
        for i in 0..=100 {
            let e = i as f32 / 100.0;
            assert_close(hlg_oetf_inverse(hlg_oetf(e)), e, 1e-5);
        }
    }

    #[test]
    fn hlg_clamps_out_of_range() {
        assert_eq!(hlg_oetf(-0.5), 0.0);
        assert_eq!(hlg_oetf(2.0), hlg_oetf(1.0));
        assert_eq!(hlg_oetf_inverse(-1.0), 0.0);
        assert_eq!(hlg_oetf_inverse(1.5), hlg_oetf_inverse(1.0));
    }
}
