//! The `.fcl` sidecar document schema (PRD §6).
//!
//! One [`SidecarDoc`] per image, holding everything needed to reproduce an
//! export bit-identically forever: schema + pipeline versions, the full
//! [`EditState`], the live-index metadata block the directory view scans,
//! and complete, explicit export recipes.
//!
//! Every struct is `#[serde(default)]`-tolerant: fields added in future
//! minor schema revisions are ignored by old readers and defaulted by new
//! readers loading old files. Renaming or re-typing a field is a schema
//! change and requires bumping [`SCHEMA_VERSION`](crate::SCHEMA_VERSION);
//! the old form must remain readable forever.

use std::path::Path;

use ciborium::Value;
use focale_core::params::EditState;
use serde::{Deserialize, Serialize};

use crate::SCHEMA_VERSION;
use crate::cde::{self, CdeError};

/// CBOR "self-described CBOR" tag (RFC 8949 §3.4.6). Every `.fcl` file is
/// wrapped in this tag so the format is identifiable from its first three
/// bytes (`d9 d9 f7`); readers also accept untagged documents.
pub const SELF_DESCRIBE_TAG: u64 = 55799;

/// Errors produced when loading or saving a sidecar file.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// CBOR (de)serialization failure.
    #[error(transparent)]
    Cde(#[from] CdeError),
    /// The document was written by a newer schema than this build knows.
    /// Per the permanent-compatibility rule the reverse never errors.
    #[error("sidecar schema version {0} is newer than supported version {SCHEMA_VERSION}")]
    FutureSchema(u32),
    /// Filesystem failure while reading or writing.
    #[error("sidecar io: {0}")]
    Io(#[from] std::io::Error),
}

/// A complete `.fcl` sidecar document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidecarDoc {
    /// Schema version that wrote this document
    /// (= [`SCHEMA_VERSION`](crate::SCHEMA_VERSION) at write time).
    pub schema_version: u32,
    /// [`focale_core::PIPELINE_VERSION`] at creation / last edit. Exports
    /// must render with this version's algorithms forever.
    pub pipeline_version: u32,
    /// The full parameter set for every pipeline stage, including masks and
    /// retouch strokes.
    pub edit: EditState,
    /// Metadata block sufficient for the directory view to build its index
    /// by scanning sidecars alone (PRD §6–7).
    pub live_index: LiveIndex,
    /// Named export recipes. Each is complete and explicit so re-running it
    /// reproduces the exported bytes.
    pub export_recipes: Vec<ExportRecipe>,
    /// Debug provenance: the Focale build that last wrote this document,
    /// `"<release>+<short git hash>"` (e.g. `"0.1.0+e258182"`, hash
    /// `"unknown"` when built outside git). `None` = written by a
    /// pre-provenance build. Re-stamped on every save; readers MUST NOT
    /// branch on it, and it is excluded from the "identical edits →
    /// identical bytes" claim (docs/sidecar-schema.md §2.2).
    pub focale_version: Option<String>,
    /// Debug provenance: OS the writer ran on (`"linux"` / `"macos"` /
    /// `"windows"`, from [`std::env::consts::OS`]). Same rules as
    /// [`Self::focale_version`].
    pub focale_platform: Option<String>,
}

impl Default for SidecarDoc {
    fn default() -> Self {
        Self::new_default(focale_core::PIPELINE_VERSION)
    }
}

impl SidecarDoc {
    /// A pristine document (default edit state, empty live index, no
    /// recipes) stamped with the current schema version and the given
    /// pipeline version.
    pub fn new_default(pipeline_version: u32) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            pipeline_version,
            edit: EditState::default(),
            live_index: LiveIndex::default(),
            export_recipes: Vec::new(),
            focale_version: None,
            focale_platform: None,
        }
    }

    /// Stamps writer provenance (build version string + platform).
    /// Applications call this immediately before every save; the fields
    /// exist solely for debugging and have no other purpose.
    pub fn set_provenance(&mut self, version: &str, platform: &str) {
        self.focale_version = Some(version.to_string());
        self.focale_platform = Some(platform.to_string());
    }

    /// Serializes to the on-disk byte form: the document wrapped in CBOR
    /// tag [`SELF_DESCRIBE_TAG`], encoded with RFC 8949 §4.2 Core
    /// Deterministic Encoding. Identical documents always yield identical
    /// bytes.
    pub fn save_to_bytes(&self) -> Result<Vec<u8>, CdeError> {
        let tree = Value::serialized(self).map_err(|e| CdeError::Serde(e.to_string()))?;
        let tagged = Value::Tag(SELF_DESCRIBE_TAG, Box::new(tree));
        let mut out = Vec::new();
        cde::write_value(&mut out, &tagged)?;
        Ok(out)
    }

    /// Deserializes from bytes. Accepts any well-formed CBOR, with or
    /// without the [`SELF_DESCRIBE_TAG`] envelope; unknown map keys are
    /// ignored (forward tolerance). Fails with
    /// [`SidecarError::FutureSchema`] if the document's `schema_version`
    /// is newer than this build supports.
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self, SidecarError> {
        let tree: Value =
            ciborium::from_reader(bytes).map_err(|e| CdeError::Malformed(e.to_string()))?;
        let tree = match tree {
            Value::Tag(SELF_DESCRIBE_TAG, inner) => *inner,
            other => other,
        };
        let doc: Self = tree
            .deserialized()
            .map_err(|e| CdeError::Serde(e.to_string()))?;
        if doc.schema_version > SCHEMA_VERSION {
            return Err(SidecarError::FutureSchema(doc.schema_version));
        }
        Ok(doc)
    }

    /// Reads and decodes the sidecar at `path`.
    pub fn load(path: &Path) -> Result<Self, SidecarError> {
        let bytes = std::fs::read(path)?;
        Self::load_from_bytes(&bytes)
    }

    /// Atomically writes the sidecar to `path`: the bytes go to a temporary
    /// file in the same directory, are synced, and the temporary file is
    /// renamed over `path`, so a reader never observes a partial sidecar.
    pub fn save(&self, path: &Path) -> Result<(), SidecarError> {
        let bytes = self.save_to_bytes()?;
        let file_name = path.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "sidecar path has no file name",
            )
        })?;
        let mut tmp_name = file_name.to_os_string();
        tmp_name.push(format!(".{}.tmp", std::process::id()));
        let tmp_path = path.parent().unwrap_or(Path::new(".")).join(tmp_name);
        {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Culling/index metadata (PRD §6): the directory view builds its entire
/// index from this block alone — file names and directory shape carry no
/// meaning.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveIndex {
    /// Star rating, 0–5.
    pub rating: u8,
    /// Pick / reject flag.
    pub flag: Flag,
    /// Colour label name (e.g. `"Red"`), `None` = unlabelled.
    pub label: Option<String>,
    /// Cached capture time from raw metadata, RFC 3339 when known.
    pub capture_time: Option<String>,
    /// SHA-256 of the last-rendered thumbnail, for cache validation.
    /// Serialized as a CBOR byte string (major type 2), not an array.
    #[serde(with = "hash_bytes")]
    pub thumbnail_hash: Option<[u8; 32]>,
}

/// Filmstrip pick/reject flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Flag {
    /// Unflagged.
    #[default]
    None,
    /// Picked.
    Pick,
    /// Rejected.
    Reject,
}

/// One named export configuration. A recipe records every option that
/// affects output bytes, explicitly — re-running a recipe against the same
/// raw + edit + pipeline version reproduces the export bit-identically.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportRecipe {
    /// User-visible recipe name.
    pub name: String,
    /// Output file format with its complete encoder options.
    pub format: ExportFormat,
    /// Output colour options.
    pub color: ExportColor,
    /// HDR output options; `None` = SDR export.
    pub hdr: Option<HdrOptions>,
    /// Output resizing; `None` = native (post-crop) resolution.
    pub resize: Option<ResizeSpec>,
}

/// Output format and its encoder options (PRD §8 codec list). Every
/// format-specific knob that affects output bytes lives here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExportFormat {
    /// 16-bit TIFF, the designated hand-off format.
    Tiff16 {
        /// Lossless compression scheme.
        compression: TiffCompression,
    },
    /// PNG.
    Png {
        /// Bits per sample: 8 or 16.
        bit_depth: u8,
    },
    /// Baseline JPEG (always 8-bit).
    Jpeg {
        /// Quality, 1–100.
        quality: u8,
    },
    /// JPEG XL.
    JpegXl {
        /// Butteraugli distance; 0.0 = mathematically lossless,
        /// 1.0 ≈ visually lossless, larger = smaller files.
        distance: f32,
        /// Bits per sample: 8 or 16.
        bit_depth: u8,
    },
    /// AVIF.
    Avif {
        /// Quality, 1–100 (100 = best).
        quality: u8,
        /// Bits per sample: 8, 10, or 12.
        bit_depth: u8,
    },
}

impl Default for ExportFormat {
    /// 16-bit deflate-compressed TIFF — the hand-off format.
    fn default() -> Self {
        Self::Tiff16 {
            compression: TiffCompression::Deflate,
        }
    }
}

/// Lossless TIFF compression schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TiffCompression {
    /// Uncompressed.
    None,
    /// Deflate (zlib).
    #[default]
    Deflate,
    /// LZW.
    Lzw,
}

/// Output colour options.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportColor {
    /// Target colour gamut.
    pub gamut: ExportGamut,
}

/// Target export gamut. Local to the schema; maps 1:1 to
/// `focale_core::color::Gamut` at export time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportGamut {
    /// sRGB / Rec.709 primaries, D65.
    #[default]
    Srgb,
    /// Display P3, D65.
    DisplayP3,
    /// Adobe RGB (1998).
    AdobeRgb,
    /// Rec.2020 (the working-space primaries).
    Rec2020,
}

/// HDR output options (PRD §5: full capability of each format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HdrOptions {
    /// Transfer function.
    pub transfer: HdrTransfer,
    /// Mastering peak luminance in cd/m² (nits).
    pub peak_nits: f32,
    /// Gain-map generation. A seam only in v1: recipes may carry the block
    /// but execution rejects it (docs/architecture.md §7).
    pub gain_map: Option<GainMapOptions>,
}

impl Default for HdrOptions {
    fn default() -> Self {
        Self {
            transfer: HdrTransfer::Pq,
            peak_nits: 1000.0,
            gain_map: None,
        }
    }
}

/// HDR transfer functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HdrTransfer {
    /// SMPTE ST 2084 perceptual quantizer.
    #[default]
    Pq,
    /// Hybrid log–gamma (ARIB STD-B67).
    Hlg,
}

/// Gain-map export options. Intentionally empty in schema v1: the block is
/// a forward seam (its presence requests a gain map, which v1 execution
/// rejects); fields arrive with the feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GainMapOptions {}

/// Output resizing. Minimal in v1: a long-edge target only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResizeSpec {
    /// Target length of the longer output edge in pixels. Never upscales:
    /// values at or above the native long edge leave the image unresized.
    pub long_edge: u32,
}

/// Serializes `Option<[u8; 32]>` as a CBOR byte string (major type 2)
/// rather than an array of 32 integers — same rationale as the
/// `serde_bytes_vec` helper in `focale_core::masks`.
mod hash_bytes {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(hash: &Option<[u8; 32]>, ser: S) -> Result<S::Ok, S::Error> {
        match hash {
            Some(h) => ser.serialize_some(&serde_bytes_ref(h)),
            None => ser.serialize_none(),
        }
    }

    /// Wraps a byte array so it serializes via `serialize_bytes`.
    fn serde_bytes_ref(bytes: &[u8; 32]) -> impl serde::Serialize + '_ {
        struct B<'a>(&'a [u8]);
        impl serde::Serialize for B<'_> {
            fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_bytes(self.0)
            }
        }
        B(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Option<[u8; 32]>, D::Error> {
        struct OptV;
        impl<'de> serde::de::Visitor<'de> for OptV {
            type Value = Option<[u8; 32]>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("null or a 32-byte string")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D2: Deserializer<'de>>(self, de: D2) -> Result<Self::Value, D2::Error> {
                de.deserialize_byte_buf(BytesV).map(Some)
            }
        }
        struct BytesV;
        impl serde::de::Visitor<'_> for BytesV {
            type Value = [u8; 32];
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a 32-byte string")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                <[u8; 32]>::try_from(v).map_err(|_| E::invalid_length(v.len(), &"32 bytes"))
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                self.visit_bytes(&v)
            }
        }
        de.deserialize_option(OptV)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_doc_is_current_versions() {
        let doc = SidecarDoc::default();
        assert_eq!(doc.schema_version, SCHEMA_VERSION);
        assert_eq!(doc.pipeline_version, focale_core::PIPELINE_VERSION);
    }

    #[test]
    fn saved_bytes_start_with_self_describe_tag() {
        let bytes = SidecarDoc::default().save_to_bytes().unwrap();
        assert_eq!(&bytes[..3], &[0xd9, 0xd9, 0xf7]);
    }

    #[test]
    fn thumbnail_hash_serializes_as_byte_string() {
        let mut doc = SidecarDoc::default();
        doc.live_index.thumbnail_hash = Some([0xAB; 32]);
        let bytes = doc.save_to_bytes().unwrap();
        // 32-byte string: head 0x58 0x20 followed by the bytes.
        let needle = {
            let mut n = vec![0x58, 0x20];
            n.extend_from_slice(&[0xAB; 32]);
            n
        };
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "hash must be encoded as CBOR major type 2"
        );
        let back = SidecarDoc::load_from_bytes(&bytes).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn default_doc_has_unknown_provenance() {
        let doc = SidecarDoc::default();
        assert_eq!(doc.focale_version, None);
        assert_eq!(doc.focale_platform, None);
    }

    #[test]
    fn set_provenance_round_trips() {
        let mut doc = SidecarDoc::default();
        doc.set_provenance("0.1.0+e258182", "linux");
        let back = SidecarDoc::load_from_bytes(&doc.save_to_bytes().unwrap()).unwrap();
        assert_eq!(back.focale_version.as_deref(), Some("0.1.0+e258182"));
        assert_eq!(back.focale_platform.as_deref(), Some("linux"));
    }

    #[test]
    fn missing_provenance_keys_default_to_none() {
        // Simulate a document written by a pre-provenance build: strip the
        // two keys from the encoded map and reload.
        let mut doc = SidecarDoc::default();
        doc.set_provenance("0.1.0+e258182", "linux");
        let tree = Value::serialized(&doc).unwrap();
        let Value::Map(entries) = tree else {
            panic!("document must encode as a map");
        };
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|(k, _)| {
                k.as_text()
                    .is_none_or(|t| t != "focale_version" && t != "focale_platform")
            })
            .collect();
        let mut bytes = Vec::new();
        cde::write_value(&mut bytes, &Value::Map(entries)).unwrap();
        let back = SidecarDoc::load_from_bytes(&bytes).unwrap();
        assert_eq!(back.focale_version, None);
        assert_eq!(back.focale_platform, None);
    }
}
