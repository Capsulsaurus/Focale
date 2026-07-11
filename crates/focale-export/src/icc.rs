//! Minimal, deterministic ICC v4 RGB matrix/TRC profile generation.
//!
//! One profile per export gamut, generated programmatically so exports
//! never depend on binary blobs and the bytes never vary:
//!
//! - **Header:** version 4.3, class `mntr`, PCS `XYZ ` with the standard
//!   D50 illuminant, **zeroed creation timestamp and zeroed profile ID**
//!   (both permitted by ICC.1:2010 §7.2.8/§7.2.18) so the bytes are
//!   identical on every run and machine.
//! - **Tags:** `desc`/`cprt` (`mluc`), `wtpt` (media white = the PCS D50
//!   values, per ICC v4 practice for display profiles), `chad` (`sf32`,
//!   Bradford D65 → D50 adaptation), `rXYZ`/`gXYZ`/`bXYZ` (the gamut's
//!   RGB→XYZ(D65) matrix columns adapted to D50 through the same Bradford
//!   matrix), and `rTRC` (`para` parametric curve) with `gTRC`/`bTRC`
//!   sharing the identical tag data (offset sharing is explicitly allowed).
//! - **Transfers:** parametric type 3 with the IEC 61966-2-1 sRGB
//!   constants for sRGB, Display P3 **and Rec.2020 (the v1 pinned SDR
//!   transfer choice — see crate docs)**; parametric type 0 with
//!   γ = 563/256 for Adobe RGB (1998).
//! - All fixed-point encoding is s15Fixed16 with the pinned rounding
//!   `floor(v · 65536 + 0.5)` computed in `f64`.
//!
//! The derivation uses the published `f32` matrix constants from
//! [`focale_core::color::primaries`] plus [`bradford_adaptation`]; the
//! s15Fixed16 quantum (≈1.5e-5) is far coarser than `f32` precision, so no
//! `f64` re-derivation is needed.

use focale_core::color::adapt::ILLUMINANT_D65;
use focale_core::color::{Gamut, Mat3, bradford_adaptation};

/// PCS illuminant (D50) X value in s15Fixed16, from ICC.1:2010 §7.2.16.
const D50_X: i32 = 0x0000_F6D6;
/// PCS illuminant (D50) Y value in s15Fixed16.
const D50_Y: i32 = 0x0001_0000;
/// PCS illuminant (D50) Z value in s15Fixed16.
const D50_Z: i32 = 0x0000_D32D;

/// The PCS illuminant as exact `f64` values (the s15Fixed16 header numbers
/// above). The Bradford adaptation targets *this* white — not the CIE
/// 4-digit D50 chromaticity, which differs by ≈3e-4 in Z — so that
/// rXYZ + gXYZ + bXYZ reproduces the PCS illuminant exactly, as ICC v4
/// requires of matrix profiles.
const ICC_D50_XYZ: [f64; 3] = [
    D50_X as f64 / 65536.0,
    D50_Y as f64 / 65536.0,
    D50_Z as f64 / 65536.0,
];

/// xy chromaticity of [`ICC_D50_XYZ`] (Y = 1, so `xy_to_xyz` of this
/// reproduces the exact PCS illuminant).
fn icc_d50_xy() -> [f64; 2] {
    let sum = ICC_D50_XYZ[0] + ICC_D50_XYZ[1] + ICC_D50_XYZ[2];
    [ICC_D50_XYZ[0] / sum, ICC_D50_XYZ[1] / sum]
}

/// s15Fixed16 encoding with the pinned rounding rule (`f64` maths,
/// `floor(v · 65536 + 0.5)`).
fn s15f16(v: f64) -> i32 {
    (v * 65536.0 + 0.5).floor() as i32
}

/// Human-readable profile description per gamut.
fn description(gamut: Gamut) -> &'static str {
    match gamut {
        Gamut::Srgb => "Focale sRGB",
        Gamut::DisplayP3 => "Focale Display P3",
        Gamut::AdobeRgb => "Focale Adobe RGB (1998) compatible",
        Gamut::Rec2020 => "Focale Rec. 2020 (sRGB transfer)",
    }
}

/// `mluc` tag data with a single `enUS` record.
fn mluc(text: &str) -> Vec<u8> {
    let mut utf16 = Vec::new();
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_be_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"mluc");
    out.extend_from_slice(&0u32.to_be_bytes()); // reserved
    out.extend_from_slice(&1u32.to_be_bytes()); // record count
    out.extend_from_slice(&12u32.to_be_bytes()); // record size
    out.extend_from_slice(b"enUS");
    out.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes()); // string offset
    out.extend_from_slice(&utf16);
    out
}

/// `XYZ ` tag data holding one XYZ number.
fn xyz_tag(xyz: [i32; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"XYZ ");
    out.extend_from_slice(&0u32.to_be_bytes());
    for v in xyz {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out
}

/// `sf32` tag data holding a 3×3 matrix in row-major order.
fn sf32_tag(m: &Mat3) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"sf32");
    out.extend_from_slice(&0u32.to_be_bytes());
    for row in &m.0 {
        for &v in row {
            out.extend_from_slice(&s15f16(f64::from(v)).to_be_bytes());
        }
    }
    out
}

/// `para` tag data: parametric curve type + s15Fixed16 parameters.
fn para_tag(function_type: u16, params: &[f64]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"para");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&function_type.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    for &p in params {
        out.extend_from_slice(&s15f16(p).to_be_bytes());
    }
    out
}

/// The transfer curve tag data for a gamut (see module docs).
fn trc_tag(gamut: Gamut) -> Vec<u8> {
    match gamut {
        // IEC 61966-2-1: Y = (aX + b)^g for X ≥ d, else Y = cX.
        Gamut::Srgb | Gamut::DisplayP3 | Gamut::Rec2020 => {
            para_tag(3, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045])
        }
        // Adobe RGB (1998): pure gamma of exactly 563/256.
        Gamut::AdobeRgb => para_tag(0, &[563.0 / 256.0]),
    }
}

/// Generates the deterministic ICC v4 profile for `gamut`.
///
/// The same input always yields the same bytes (zeroed timestamp and
/// profile ID; all values derived from published constants).
pub fn profile(gamut: Gamut) -> Vec<u8> {
    let chad = bradford_adaptation(ILLUMINANT_D65, icc_d50_xy());
    let rgb_to_xyz = gamut.rgb_to_xyz();
    // Column j of RGB→XYZ(D65) is primary j's XYZ; adapt each to D50.
    let column = |j: usize| -> [i32; 3] {
        let col = [rgb_to_xyz.0[0][j], rgb_to_xyz.0[1][j], rgb_to_xyz.0[2][j]];
        let adapted = chad.mul_vec(col);
        [
            s15f16(f64::from(adapted[0])),
            s15f16(f64::from(adapted[1])),
            s15f16(f64::from(adapted[2])),
        ]
    };

    let trc = trc_tag(gamut);
    // Tag order pinned: desc, cprt, wtpt, chad, rXYZ, gXYZ, bXYZ, rTRC,
    // gTRC, bTRC (the three TRCs share one data block).
    let blocks: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", mluc(description(gamut))),
        (b"cprt", mluc("Public domain; no copyright")),
        (b"wtpt", xyz_tag([D50_X, D50_Y, D50_Z])),
        (b"chad", sf32_tag(&chad)),
        (b"rXYZ", xyz_tag(column(0))),
        (b"gXYZ", xyz_tag(column(1))),
        (b"bXYZ", xyz_tag(column(2))),
        (b"rTRC", trc),
    ];
    let shared_trc: [&[u8; 4]; 2] = [b"gTRC", b"bTRC"];
    let tag_count = blocks.len() + shared_trc.len();

    // Layout: 128-byte header, tag table, then 4-byte-aligned tag data.
    let table_len = 4 + 12 * tag_count;
    let mut data_offset = 128 + table_len as u32;
    let mut table = Vec::with_capacity(table_len);
    table.extend_from_slice(&(tag_count as u32).to_be_bytes());
    let mut data = Vec::new();
    let mut trc_entry = (0u32, 0u32);
    for (sig, block) in &blocks {
        table.extend_from_slice(*sig);
        table.extend_from_slice(&data_offset.to_be_bytes());
        table.extend_from_slice(&(block.len() as u32).to_be_bytes());
        if *sig == b"rTRC" {
            trc_entry = (data_offset, block.len() as u32);
        }
        data.extend_from_slice(block);
        let padded = block.len().next_multiple_of(4);
        data.resize(data.len() + (padded - block.len()), 0);
        data_offset += padded as u32;
    }
    for sig in shared_trc {
        table.extend_from_slice(sig);
        table.extend_from_slice(&trc_entry.0.to_be_bytes());
        table.extend_from_slice(&trc_entry.1.to_be_bytes());
    }

    let size = 128 + table.len() + data.len();
    let mut out = Vec::with_capacity(size);
    // Header (128 bytes, ICC.1:2010 §7.2).
    out.extend_from_slice(&(size as u32).to_be_bytes()); // profile size
    out.extend_from_slice(&[0; 4]); // preferred CMM: none
    out.extend_from_slice(&0x0430_0000u32.to_be_bytes()); // version 4.3
    out.extend_from_slice(b"mntr"); // display device class
    out.extend_from_slice(b"RGB "); // data colour space
    out.extend_from_slice(b"XYZ "); // PCS
    out.extend_from_slice(&[0; 12]); // creation date-time: zeroed (pinned)
    out.extend_from_slice(b"acsp"); // profile file signature
    out.extend_from_slice(&[0; 4]); // primary platform: none
    out.extend_from_slice(&[0; 4]); // flags
    out.extend_from_slice(&[0; 4]); // device manufacturer
    out.extend_from_slice(&[0; 4]); // device model
    out.extend_from_slice(&[0; 8]); // device attributes
    out.extend_from_slice(&0u32.to_be_bytes()); // rendering intent: perceptual
    out.extend_from_slice(&D50_X.to_be_bytes()); // PCS illuminant
    out.extend_from_slice(&D50_Y.to_be_bytes());
    out.extend_from_slice(&D50_Z.to_be_bytes());
    out.extend_from_slice(&[0; 4]); // profile creator
    out.extend_from_slice(&[0; 16]); // profile ID: zeroed (pinned)
    out.extend_from_slice(&[0; 28]); // reserved
    debug_assert_eq!(out.len(), 128);

    out.extend_from_slice(&table);
    out.extend_from_slice(&data);
    debug_assert_eq!(out.len(), size);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn be_i32(bytes: &[u8], at: usize) -> i32 {
        i32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    /// Finds a tag's (offset, size) in the tag table.
    fn find_tag(profile: &[u8], sig: &[u8; 4]) -> (usize, usize) {
        let count = be_u32(profile, 128) as usize;
        for i in 0..count {
            let at = 132 + 12 * i;
            if &profile[at..at + 4] == sig {
                return (
                    be_u32(profile, at + 4) as usize,
                    be_u32(profile, at + 8) as usize,
                );
            }
        }
        panic!("tag {sig:?} missing");
    }

    #[test]
    fn header_fields_round_trip() {
        for gamut in Gamut::ALL {
            let p = profile(gamut);
            assert_eq!(be_u32(&p, 0) as usize, p.len(), "size field");
            assert_eq!(&p[12..16], b"mntr");
            assert_eq!(&p[16..20], b"RGB ");
            assert_eq!(&p[20..24], b"XYZ ");
            assert_eq!(&p[36..40], b"acsp");
            assert_eq!(&p[24..36], &[0u8; 12], "timestamp zeroed");
            assert_eq!(&p[84..100], &[0u8; 16], "profile ID zeroed");
            assert_eq!(be_i32(&p, 68), D50_X);
            assert_eq!(p.len() % 4, 0, "4-byte aligned");
            assert!(p.len() < 2048, "minimal profile stays small");
        }
    }

    #[test]
    fn primaries_sum_to_d50_white() {
        // rXYZ + gXYZ + bXYZ must reproduce the (adapted) white point:
        // the D50 PCS illuminant, within s15Fixed16 rounding.
        for gamut in Gamut::ALL {
            let p = profile(gamut);
            let mut sum = [0i64; 3];
            for sig in [b"rXYZ", b"gXYZ", b"bXYZ"] {
                let (off, size) = find_tag(&p, sig);
                assert_eq!(size, 20);
                assert_eq!(&p[off..off + 4], b"XYZ ");
                for (i, s) in sum.iter_mut().enumerate() {
                    *s += i64::from(be_i32(&p, off + 8 + 4 * i));
                }
            }
            for (s, d50) in sum.iter().zip([D50_X, D50_Y, D50_Z]) {
                assert!(
                    (s - i64::from(d50)).abs() <= 16,
                    "{gamut:?}: white sum {s} vs D50 {d50}"
                );
            }
        }
    }

    #[test]
    fn trc_matches_gamut() {
        let p = profile(Gamut::AdobeRgb);
        let (off, size) = find_tag(&p, b"rTRC");
        assert_eq!(size, 16); // para type 0: header + one parameter
        assert_eq!(u16::from_be_bytes([p[off + 8], p[off + 9]]), 0);
        // γ = 563/256 is exact in s15Fixed16: 563 · 256 = 144128.
        assert_eq!(be_i32(&p, off + 12), 144_128);

        let p = profile(Gamut::Srgb);
        let (off, size) = find_tag(&p, b"rTRC");
        assert_eq!(size, 32); // para type 3: header + five parameters
        assert_eq!(u16::from_be_bytes([p[off + 8], p[off + 9]]), 3);
        assert_eq!(be_i32(&p, off + 12), s15f16(2.4));
        // gTRC/bTRC share rTRC's data block.
        assert_eq!(find_tag(&p, b"gTRC"), (off, size));
        assert_eq!(find_tag(&p, b"bTRC"), (off, size));
    }

    #[test]
    fn deterministic_bytes() {
        for gamut in Gamut::ALL {
            assert_eq!(profile(gamut), profile(gamut));
        }
    }
}
