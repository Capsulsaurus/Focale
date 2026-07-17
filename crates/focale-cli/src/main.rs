//! Headless Focale: deterministic export from the command line.
//!
//! This binary is the reference export path. CI renders the committed
//! fixtures on x86_64 and aarch64 and byte-compares the results (PRD §10,
//! docs/architecture.md §11).
//!
//! ```text
//! focale-cli render <raw> [--sidecar <file.fcl>] [--format <f>]
//!                   [--gamut <g>] [--hdr pq|hlg] [--out <file>] [--hash]
//! focale-cli hash <raw> [--sidecar <file.fcl>]      # working-space hash
//! focale-cli version
//! ```
//!
//! Formats: `tiff16`, `png16`, `png8`, `jpeg`, `jxl`, `avif`.
//! Gamuts: `srgb`, `display-p3`, `adobe-rgb`, `rec2020`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use focale_sidecar::SidecarDoc;
use focale_sidecar::schema::{
    ExportColor, ExportFormat, ExportGamut, ExportRecipe, HdrOptions, HdrTransfer, TiffCompression,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("render") => cmd_render(&args[1..]),
        Some("hash") => cmd_hash(&args[1..]),
        Some("version") => {
            println!("{}", version_line());
            Ok(())
        }
        _ => {
            eprintln!("usage: focale-cli <render|hash|version> …  (see --help in docs)");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn version_line() -> String {
    format!(
        "focale-cli {} (pipeline v{}, sidecar schema v{})",
        focale_buildinfo::version(),
        focale_core::PIPELINE_VERSION,
        focale_sidecar::SCHEMA_VERSION,
    )
}

/// Parsed `--key value` options.
struct Opts {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

fn parse(args: &[String]) -> Opts {
    let mut positional = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(name) = args[i].strip_prefix("--") {
            let value = if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                i += 1;
                Some(args[i].clone())
            } else {
                None
            };
            flags.push((name.to_string(), value));
        } else {
            positional.push(args[i].clone());
        }
        i += 1;
    }
    Opts { positional, flags }
}

impl Opts {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }
    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
}

/// Loads the sidecar next to `raw` (or an explicit `--sidecar` path); falls
/// back to defaults when absent.
fn load_doc(raw: &Path, explicit: Option<&str>) -> Result<SidecarDoc, String> {
    let path = explicit
        .map(PathBuf::from)
        .unwrap_or_else(|| focale_sidecar::sidecar_path_for(raw));
    if path.exists() {
        SidecarDoc::load(&path).map_err(|e| format!("sidecar {}: {e}", path.display()))
    } else if explicit.is_some() {
        Err(format!("sidecar not found: {}", path.display()))
    } else {
        Ok(SidecarDoc::new_default(focale_core::PIPELINE_VERSION))
    }
}

fn render_working(
    raw: &Path,
    doc: &SidecarDoc,
) -> Result<focale_core::pipeline::RenderOutput, String> {
    let decoded = focale_core::decode::decode_file(raw).map_err(|e| e.to_string())?;
    let input = focale_core::pipeline::RenderInput {
        decoded: &decoded,
        edit: &doc.edit,
        scale: 1.0,
    };
    focale_core::pipeline::render(&input, doc.pipeline_version).map_err(|e| e.to_string())
}

fn gamut_from(name: &str) -> Result<ExportGamut, String> {
    match name {
        "srgb" => Ok(ExportGamut::Srgb),
        "display-p3" => Ok(ExportGamut::DisplayP3),
        "adobe-rgb" => Ok(ExportGamut::AdobeRgb),
        "rec2020" => Ok(ExportGamut::Rec2020),
        other => Err(format!("unknown gamut: {other}")),
    }
}

fn format_from(name: &str) -> Result<ExportFormat, String> {
    match name {
        "tiff16" => Ok(ExportFormat::Tiff16 {
            compression: TiffCompression::Deflate,
        }),
        "png16" => Ok(ExportFormat::Png { bit_depth: 16 }),
        "png8" => Ok(ExportFormat::Png { bit_depth: 8 }),
        "jpeg" => Ok(ExportFormat::Jpeg { quality: 92 }),
        "jxl" => Ok(ExportFormat::JpegXl {
            distance: 1.0,
            bit_depth: 16,
        }),
        "avif" => Ok(ExportFormat::Avif {
            quality: 80,
            bit_depth: 10,
        }),
        other => Err(format!("unknown format: {other}")),
    }
}

fn extension(format: &ExportFormat) -> &'static str {
    match format {
        ExportFormat::Tiff16 { .. } => "tif",
        ExportFormat::Png { .. } => "png",
        ExportFormat::Jpeg { .. } => "jpg",
        ExportFormat::JpegXl { .. } => "jxl",
        ExportFormat::Avif { .. } => "avif",
    }
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let opts = parse(args);
    let raw = PathBuf::from(
        opts.positional
            .first()
            .ok_or("render: missing <raw> argument")?,
    );
    let doc = load_doc(&raw, opts.flag("sidecar"))?;
    let format = format_from(opts.flag("format").unwrap_or("tiff16"))?;
    let gamut = gamut_from(opts.flag("gamut").unwrap_or("srgb"))?;
    let hdr = match opts.flag("hdr") {
        None => None,
        Some("pq") => Some(HdrTransfer::Pq),
        Some("hlg") => Some(HdrTransfer::Hlg),
        Some(other) => return Err(format!("unknown hdr transfer: {other}")),
    };
    let recipe = ExportRecipe {
        name: "cli".into(),
        format,
        color: ExportColor { gamut },
        hdr: hdr.map(|transfer| HdrOptions {
            transfer,
            peak_nits: 1000.0,
            gain_map: None,
        }),
        resize: None,
    };

    let out = render_working(&raw, &doc)?;
    for w in &out.warnings {
        eprintln!("warning: {w:?}");
    }
    let bytes = focale_export::encode(&out.image, &recipe).map_err(|e| e.to_string())?;

    if opts.has("hash") {
        println!("{}", hex(&sha256(&bytes)));
    }
    let out_path = opts.flag("out").map(PathBuf::from).unwrap_or_else(|| {
        let stem = raw.file_stem().map(|s| s.to_string_lossy().into_owned());
        PathBuf::from(format!(
            "{}.{}",
            stem.unwrap_or_else(|| "out".into()),
            extension(&recipe.format)
        ))
    });
    std::fs::write(&out_path, &bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
    eprintln!("wrote {} ({} bytes)", out_path.display(), bytes.len());
    Ok(())
}

/// Hashes the working-space image (pre-encode): pinpoints whether a
/// determinism break is in the pipeline or in a codec.
fn cmd_hash(args: &[String]) -> Result<(), String> {
    let opts = parse(args);
    let raw = PathBuf::from(
        opts.positional
            .first()
            .ok_or("hash: missing <raw> argument")?,
    );
    let doc = load_doc(&raw, opts.flag("sidecar"))?;
    let out = render_working(&raw, &doc)?;
    let mut bytes = Vec::with_capacity(out.image.data().len() * 4);
    for v in out.image.data() {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    println!(
        "{}  {}x{}",
        hex(&sha256(&bytes)),
        out.image.width(),
        out.image.height()
    );
    Ok(())
}

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_reports_pipeline_and_schema() {
        let line = version_line();
        assert!(line.contains("pipeline v1"));
        assert!(line.contains("sidecar schema v1"));
    }

    #[test]
    fn parse_flags_and_positionals() {
        let args: Vec<String> = ["a.arw", "--format", "png16", "--hash"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = parse(&args);
        assert_eq!(o.positional, vec!["a.arw"]);
        assert_eq!(o.flag("format"), Some("png16"));
        assert!(o.has("hash"));
        assert!(!o.has("gamut"));
    }

    #[test]
    fn format_and_gamut_parsing() {
        assert!(format_from("tiff16").is_ok());
        assert!(format_from("bmp").is_err());
        assert!(gamut_from("rec2020").is_ok());
        assert!(gamut_from("ntsc").is_err());
    }
}
