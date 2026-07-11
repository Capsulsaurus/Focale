//! RFC 8949 §4.2 Core Deterministic Encoding of CBOR.
//!
//! Sidecars must serialize identical edits to identical bytes, forever.
//! Rather than trusting a third-party encoder's byte output to stay stable
//! across its own releases, values pass through [`ciborium::Value`] (for
//! serde ergonomics) and the bytes are produced by this module's canonical
//! writer:
//!
//! - integers and lengths in shortest form, definite lengths only
//! - map keys sorted by bytewise lexicographic order of their encoding
//! - floats in the shortest of f16/f32/f64 that preserves the value,
//!   with NaN canonicalized to `0xf9 0x7e 0x00`
//!
//! Decoding accepts any well-formed CBOR (we only guarantee what we write),
//! via `ciborium`'s reader.

use ciborium::Value;
use ciborium::value::Integer;

/// Errors produced when encoding or decoding sidecar CBOR.
#[derive(Debug, thiserror::Error)]
pub enum CdeError {
    /// A value cannot be represented in deterministic CBOR.
    #[error("unencodable value: {0}")]
    Unencodable(String),
    /// The input bytes are not well-formed CBOR.
    #[error("malformed CBOR: {0}")]
    Malformed(String),
    /// Serde-level (de)serialization failure.
    #[error("serde: {0}")]
    Serde(String),
}

/// Serializes `value` to Core Deterministic Encoding bytes.
pub fn to_deterministic_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, CdeError> {
    let tree = Value::serialized(value).map_err(|e| CdeError::Serde(e.to_string()))?;
    let mut out = Vec::new();
    write_value(&mut out, &tree)?;
    Ok(out)
}

/// Deserializes a `T` from CBOR bytes (any well-formed encoding accepted).
pub fn from_bytes<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, CdeError> {
    let tree: Value =
        ciborium::from_reader(bytes).map_err(|e| CdeError::Malformed(e.to_string()))?;
    tree.deserialized()
        .map_err(|e| CdeError::Serde(e.to_string()))
}

/// Encodes a [`Value`] tree with Core Deterministic Encoding.
pub fn write_value(out: &mut Vec<u8>, value: &Value) -> Result<(), CdeError> {
    match value {
        Value::Integer(i) => write_integer(out, *i),
        Value::Bytes(b) => {
            write_head(out, 2, b.len() as u64);
            out.extend_from_slice(b);
            Ok(())
        }
        Value::Text(s) => {
            write_head(out, 3, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
            Ok(())
        }
        Value::Array(items) => {
            write_head(out, 4, items.len() as u64);
            for item in items {
                write_value(out, item)?;
            }
            Ok(())
        }
        Value::Map(entries) => write_map(out, entries),
        Value::Tag(tag, inner) => {
            write_head(out, 6, *tag);
            write_value(out, inner)
        }
        Value::Bool(false) => {
            out.push(0xf4);
            Ok(())
        }
        Value::Bool(true) => {
            out.push(0xf5);
            Ok(())
        }
        Value::Null => {
            out.push(0xf6);
            Ok(())
        }
        Value::Float(f) => {
            write_float(out, *f);
            Ok(())
        }
        other => Err(CdeError::Unencodable(format!("{other:?}"))),
    }
}

/// Writes a major type + shortest-form argument.
fn write_head(out: &mut Vec<u8>, major: u8, arg: u64) {
    let mt = major << 5;
    match arg {
        0..=23 => out.push(mt | arg as u8),
        24..=0xff => {
            out.push(mt | 24);
            out.push(arg as u8);
        }
        0x100..=0xffff => {
            out.push(mt | 25);
            out.extend_from_slice(&(arg as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(mt | 26);
            out.extend_from_slice(&(arg as u32).to_be_bytes());
        }
        _ => {
            out.push(mt | 27);
            out.extend_from_slice(&arg.to_be_bytes());
        }
    }
}

fn write_integer(out: &mut Vec<u8>, i: Integer) -> Result<(), CdeError> {
    let v = i128::from(i);
    if v >= 0 {
        let v = u64::try_from(v).map_err(|_| CdeError::Unencodable(format!("integer {v}")))?;
        write_head(out, 0, v);
    } else {
        let magnitude =
            u64::try_from(-1 - v).map_err(|_| CdeError::Unencodable(format!("integer {v}")))?;
        write_head(out, 1, magnitude);
    }
    Ok(())
}

/// Writes a float in the shortest of f16/f32/f64 that preserves the value.
fn write_float(out: &mut Vec<u8>, f: f64) {
    if f.is_nan() {
        // Canonical NaN.
        out.extend_from_slice(&[0xf9, 0x7e, 0x00]);
        return;
    }
    let f16 = half::f16::from_f64(f);
    if f64::from(f16) == f {
        out.push(0xf9);
        out.extend_from_slice(&f16.to_be_bytes());
        return;
    }
    let f32v = f as f32;
    if f64::from(f32v) == f {
        out.push(0xfa);
        out.extend_from_slice(&f32v.to_be_bytes());
        return;
    }
    out.push(0xfb);
    out.extend_from_slice(&f.to_be_bytes());
}

/// Writes a map with keys sorted bytewise by their deterministic encoding.
fn write_map(out: &mut Vec<u8>, entries: &[(Value, Value)]) -> Result<(), CdeError> {
    let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let mut ek = Vec::new();
        write_value(&mut ek, k)?;
        let mut ev = Vec::new();
        write_value(&mut ev, v)?;
        encoded.push((ek, ev));
    }
    encoded.sort_by(|a, b| a.0.cmp(&b.0));
    for window in encoded.windows(2) {
        if window[0].0 == window[1].0 {
            return Err(CdeError::Unencodable("duplicate map key".into()));
        }
    }
    write_head(out, 5, encoded.len() as u64);
    for (k, v) in encoded {
        out.extend_from_slice(&k);
        out.extend_from_slice(&v);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    fn bytes_of<T: serde::Serialize>(v: &T) -> Vec<u8> {
        to_deterministic_bytes(v).unwrap()
    }

    #[test]
    fn integers_use_shortest_form() {
        assert_eq!(bytes_of(&0u8), [0x00]);
        assert_eq!(bytes_of(&23u8), [0x17]);
        assert_eq!(bytes_of(&24u8), [0x18, 24]);
        assert_eq!(bytes_of(&255u32), [0x18, 0xff]);
        assert_eq!(bytes_of(&256u32), [0x19, 0x01, 0x00]);
        assert_eq!(bytes_of(&65536u64), [0x1a, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(bytes_of(&-1i32), [0x20]);
        assert_eq!(bytes_of(&-25i32), [0x38, 24]);
    }

    #[test]
    fn floats_use_shortest_roundtripping_width() {
        // 1.5 fits in f16.
        assert_eq!(bytes_of(&1.5f32), [0xf9, 0x3e, 0x00]);
        // 0.1f32 does not fit in f16 but fits in f32.
        assert_eq!(bytes_of(&0.1f32), [0xfa, 0x3d, 0xcc, 0xcc, 0xcd]);
        // 0.1f64 needs full f64.
        let b = bytes_of(&0.1f64);
        assert_eq!(b[0], 0xfb);
        // NaN is canonical.
        assert_eq!(bytes_of(&f32::NAN), [0xf9, 0x7e, 0x00]);
        // Infinities fit in f16.
        assert_eq!(bytes_of(&f64::INFINITY), [0xf9, 0x7c, 0x00]);
    }

    #[test]
    fn map_keys_sort_bytewise() {
        // BTreeMap of text keys: "b" < "aa" bytewise? No: sorting is on the
        // ENCODED key, and shorter strings encode with a smaller head, so
        // "b" (0x61 0x62) sorts after "aa"? 0x62 'aa' -> [0x62, 0x61, 0x61],
        // 'b' -> [0x61, 0x62]. 0x61 < 0x62 so "b" comes first.
        use std::collections::BTreeMap;
        let mut m = BTreeMap::new();
        m.insert("aa".to_string(), 1u8);
        m.insert("b".to_string(), 2u8);
        let b = bytes_of(&m);
        assert_eq!(
            b,
            [0xa2, 0x61, 0x62, 0x02, 0x62, 0x61, 0x61, 0x01],
            "one-byte key must sort before two-byte key"
        );
    }

    #[test]
    fn duplicate_keys_rejected() {
        let entries = vec![
            (Value::Text("k".into()), Value::Bool(true)),
            (Value::Text("k".into()), Value::Bool(false)),
        ];
        let mut out = Vec::new();
        assert!(write_map(&mut out, &entries).is_err());
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Nested {
        gamma: f32,
        alpha: Vec<u32>,
        beta: Option<String>,
    }

    #[test]
    fn struct_roundtrip_is_stable_and_sorted() {
        let v = Nested {
            gamma: 2.2,
            alpha: vec![1, 2, 3],
            beta: Some("x".into()),
        };
        let b1 = bytes_of(&v);
        let b2 = bytes_of(&v);
        assert_eq!(b1, b2);
        // Field names are reordered: alpha < beta < gamma bytewise.
        let decoded: Nested = from_bytes(&b1).unwrap();
        assert_eq!(decoded, v);
        // "alpha" must appear before "gamma" in the bytes.
        let pos_alpha = b1.windows(5).position(|w| w == b"alpha").unwrap();
        let pos_gamma = b1.windows(5).position(|w| w == b"gamma").unwrap();
        assert!(pos_alpha < pos_gamma);
    }
}
