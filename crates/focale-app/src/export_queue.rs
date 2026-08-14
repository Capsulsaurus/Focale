//! Background export queue (docs/subsystems/app.md batch).

use std::path::PathBuf;

use focale_sidecar::schema::{ExportColor, ExportFormat, ExportGamut, ExportRecipe};

/// Status of one queued export.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportStatus {
    /// Waiting or running.
    Pending,
    /// Written to `PathBuf`.
    Done(PathBuf),
    /// Failed with a message.
    Failed(String),
}

/// One export queue row shown in the UI.
#[derive(Debug, Clone)]
pub struct ExportItem {
    /// Source raw path.
    pub source: PathBuf,
    /// Recipe name used.
    pub recipe: String,
    /// Current status.
    pub status: ExportStatus,
}

/// The built-in starter recipes (users can add more per sidecar).
pub fn default_recipes() -> Vec<ExportRecipe> {
    vec![
        ExportRecipe {
            name: "TIFF 16-bit (hand-off)".into(),
            format: ExportFormat::Tiff16 {
                compression: focale_sidecar::schema::TiffCompression::Deflate,
            },
            color: ExportColor {
                gamut: ExportGamut::AdobeRgb,
            },
            hdr: None,
            resize: None,
        },
        ExportRecipe {
            name: "JPEG sRGB".into(),
            format: ExportFormat::Jpeg { quality: 92 },
            color: ExportColor {
                gamut: ExportGamut::Srgb,
            },
            hdr: None,
            resize: None,
        },
        ExportRecipe {
            name: "JPEG XL".into(),
            format: ExportFormat::JpegXl {
                distance: 1.0,
                bit_depth: 16,
            },
            color: ExportColor {
                gamut: ExportGamut::DisplayP3,
            },
            hdr: None,
            resize: None,
        },
        ExportRecipe {
            name: "AVIF HDR (PQ)".into(),
            format: ExportFormat::Avif {
                quality: 80,
                bit_depth: 10,
            },
            color: ExportColor {
                gamut: ExportGamut::Rec2020,
            },
            hdr: Some(focale_sidecar::schema::HdrOptions {
                transfer: focale_sidecar::schema::HdrTransfer::Pq,
                peak_nits: 1000.0,
                gain_map: None,
            }),
            resize: None,
        },
        ExportRecipe {
            name: "PNG 16-bit".into(),
            format: ExportFormat::Png { bit_depth: 16 },
            color: ExportColor {
                gamut: ExportGamut::Srgb,
            },
            hdr: None,
            resize: None,
        },
    ]
}

/// File extension for a recipe's format.
pub fn extension(format: &ExportFormat) -> &'static str {
    match format {
        ExportFormat::Tiff16 { .. } => "tif",
        ExportFormat::Png { .. } => "png",
        ExportFormat::Jpeg { .. } => "jpg",
        ExportFormat::JpegXl { .. } => "jxl",
        ExportFormat::Avif { .. } => "avif",
    }
}

/// Output path: `<dir>/focale-export/<stem>.<ext>` next to the source.
pub fn output_path(source: &std::path::Path, format: &ExportFormat) -> PathBuf {
    let dir = source
        .parent()
        .map(|p| p.join("focale-export"))
        .unwrap_or_else(|| PathBuf::from("focale-export"));
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    dir.join(format!("{stem}.{}", extension(format)))
}
