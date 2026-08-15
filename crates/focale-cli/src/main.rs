//! Headless Focale: deterministic export from the command line.
//!
//! This binary is the reference export path. CI renders the committed
//! fixtures on x86_64 and aarch64 and byte-compares the results (docs/verification.md).
//!
//! ```text
//! focale-cli render <raw> [--sidecar <file.fcl>] [--format <f>]
//!                   [--gamut <g>] [--hdr pq|hlg] [--out <file>] [--hash]
//! focale-cli hash <raw> [--sidecar <file.fcl>]      # working-space hash
//! focale-cli bench-preview [<raw>] [--sidecar <file.fcl>]
//!                   [--synthetic <WxH>] [--runs <n>] [--warmup <n>]
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
        Some("bench-preview") => cmd_bench_preview(&args[1..]),
        Some("version") => {
            println!("{}", version_line());
            Ok(())
        }
        _ => {
            eprintln!(
                "usage: focale-cli <render|hash|bench-preview|version> …  (see --help in docs)"
            );
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

/// Default synthetic preview base: the 3:2 frame whose long edge is exactly
/// [`focale_core::preview::PREVIEW_LONG_EDGE`], i.e. the largest base the app
/// ever renders interactively.
const DEFAULT_SYNTHETIC: (u32, u32) = (2560, 1707);

/// Benchmarks the preview-base pipeline — the slider-to-screen budget's CPU
/// half (issue #11; budget in `docs/subsystems/platform.md`, recorded numbers
/// in `docs/verification.md`).
///
/// This measures exactly what the app measures as its `pipeline` segment: the
/// same `focale_core::preview::render` on the same preview base. It does not
/// include scheduler queueing, GPU upload, or present, which only exist in
/// the running app.
///
/// With no `<raw>` it renders a deterministic synthetic base, so the
/// benchmark reproduces on any machine without a raw file.
fn cmd_bench_preview(args: &[String]) -> Result<(), String> {
    let opts = parse(args);
    let runs: usize = parse_count(opts.flag("runs"), 20, "runs")?;
    let warmup: usize = parse_count(opts.flag("warmup"), 3, "warmup")?;

    let (base, source) = match opts.positional.first() {
        Some(raw) => {
            let raw = PathBuf::from(raw);
            let base = focale_core::preview::build_base(&raw).map_err(|e| e.to_string())?;
            (base, format!("{}", raw.display()))
        }
        None => {
            let (w, h) = match opts.flag("synthetic") {
                None => DEFAULT_SYNTHETIC,
                Some(spec) => parse_size(spec)?,
            };
            let base = focale_core::preview::base_from_decoded(
                PathBuf::from("synthetic"),
                synthetic_raw(w, h),
            );
            (base, format!("synthetic {w}x{h}"))
        }
    };

    // The sidecar supplies the edit. `--sidecar` is resolved against the raw
    // when one was given; a synthetic base has no neighbouring sidecar, so an
    // explicit path is the only way to get a non-default edit.
    let doc = match (opts.flag("sidecar"), opts.positional.first()) {
        (Some(explicit), _) => load_doc(Path::new(explicit), Some(explicit))?,
        (None, Some(raw)) => load_doc(Path::new(raw), None)?,
        (None, None) => SidecarDoc::new_default(focale_core::PIPELINE_VERSION),
    };

    let (w, h) = (base.decoded.width, base.decoded.height);
    eprintln!(
        "preview base {w}x{h} (scale {:.3}) from {source}\n\
         edit: pipeline v{}, {} runs after {} warmup",
        base.scale, doc.pipeline_version, runs, warmup
    );

    for _ in 0..warmup {
        focale_core::preview::render(&base, &doc.edit, doc.pipeline_version, 0)
            .map_err(|e| e.to_string())?;
    }

    let mut times_ms = Vec::with_capacity(runs);
    for i in 0..runs {
        let t0 = std::time::Instant::now();
        focale_core::preview::render(&base, &doc.edit, doc.pipeline_version, i as u64)
            .map_err(|e| e.to_string())?;
        times_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let stats = Stats::of(&times_ms);
    println!(
        "min {:.1} ms  median {:.1} ms  mean {:.1} ms  max {:.1} ms  (n={runs}, {w}x{h})",
        stats.min, stats.median, stats.mean, stats.max
    );

    if opts.has("breakdown") {
        print_breakdown(&base, &doc, runs.clamp(3, 10))?;
    }

    // The 100 ms figure is the *whole* slider-to-screen budget; the pipeline
    // is only its largest part, so this verdict is a leading indicator, not
    // the measurement of record. The app's own instrumentation is.
    if stats.median > 100.0 {
        println!("OVER the 100 ms slider-to-screen budget on the pipeline alone");
    } else {
        println!(
            "within budget: pipeline uses {:.0}% of the 100 ms slider-to-screen budget",
            stats.median
        );
    }
    Ok(())
}

/// Attributes cost to stages by re-rendering with one stage disabled at a
/// time and reporting the drop against the full edit.
///
/// Two deliberate choices, both learned the hard way on a loaded machine:
///
/// - **Interleaved.** Every round measures the full edit *and* each variant,
///   so a burst of unrelated CPU load lands on all configurations rather
///   than inflating whichever one happened to run during it. Measuring each
///   variant in its own block produced deltas that were pure drift.
/// - **Compared on minima.** Noise is strictly additive for CPU-bound work,
///   so the fastest observed run is the cleanest estimate of a
///   configuration's cost. Medians are the right summary for the latency a
///   user experiences (and that is what the headline reports); minima are
///   the right basis for comparing two configurations.
///
/// This remains a subtractive estimate, not a profile: stages interact —
/// disabling geometry changes how many pixels finishing sees — so the deltas
/// need not sum to the total. It exists to point the per-stage-caching
/// follow-up at the right stages, which is what issue #11 asks for.
fn print_breakdown(
    base: &focale_core::preview::PreviewBase,
    doc: &SidecarDoc,
    rounds: usize,
) -> Result<(), String> {
    let time_once = |edit: &focale_core::params::EditState, seed: u64| -> Result<f64, String> {
        let t0 = std::time::Instant::now();
        focale_core::preview::render(base, edit, doc.pipeline_version, seed)
            .map_err(|e| e.to_string())?;
        Ok(t0.elapsed().as_secs_f64() * 1000.0)
    };

    // A control that changes nothing. Its measured "saving" is by
    // construction zero, so whatever it reads is this run's noise floor —
    // the resolution below which a stage's delta means nothing. Without it
    // a reader cannot tell a real 10 ms stage from a busy CPU.
    let mut variants: Vec<(&str, focale_core::params::EditState)> =
        vec![("(control)", doc.edit.clone())];
    variants.extend(stage_disablers().into_iter().map(|(name, disable)| {
        let mut edit = doc.edit.clone();
        disable(&mut edit);
        (name, edit)
    }));

    let mut full_samples = Vec::with_capacity(rounds);
    let mut variant_samples: Vec<Vec<f64>> = vec![Vec::with_capacity(rounds); variants.len()];
    for r in 0..rounds {
        full_samples.push(time_once(&doc.edit, r as u64)?);
        // Rotate the order every round. A fixed order gives every variant a
        // fixed position within the round, and position is not neutral —
        // clock ramp and cache state make later slots systematically slower,
        // which showed up as stages "saving" negative time. Rotation is
        // deterministic (unlike shuffling), so the benchmark stays
        // reproducible while no variant keeps a favourable slot.
        for offset in 0..variants.len() {
            let i = (offset + r) % variants.len();
            variant_samples[i].push(time_once(&variants[i].1, r as u64)?);
        }
    }

    let full_min = Stats::of(&full_samples).min;
    let mut rows: Vec<(&str, f64)> = variants
        .iter()
        .zip(&variant_samples)
        .map(|((name, _), samples)| (*name, full_min - Stats::of(samples).min))
        .collect();
    // The noise floor is how far the control drifted from zero in either
    // direction; anything smaller is unresolvable on this machine right now.
    let noise_floor = rows
        .iter()
        .find(|(name, _)| *name == "(control)")
        .map(|(_, d)| d.abs())
        .unwrap_or(0.0);
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!(
        "\nper-stage cost (ms saved by disabling that stage alone; \
         interleaved, best-of-{rounds}, full edit = {full_min:.1} ms;\n\
         noise floor {noise_floor:.1} ms — rows within it are unresolved):"
    );
    for (name, saved) in rows {
        let marker = if name == "(control)" {
            "  ← noise floor"
        } else if saved.abs() <= noise_floor {
            "  (unresolved)"
        } else if saved < 0.0 {
            // Disabling a stage cannot genuinely make the pipeline slower,
            // so a negative beyond the control's drift means this stage's
            // cost is lost in systematic error, not that it is free.
            "  (no measurable cost)"
        } else {
            ""
        };
        println!("  {name:<10} {saved:>7.1}{marker}");
    }
    Ok(())
}

/// One disabler per pipeline stage that carries an enable flag or a
/// collection the stage iterates.
#[allow(clippy::type_complexity)]
fn stage_disablers() -> Vec<(&'static str, fn(&mut focale_core::params::EditState))> {
    vec![
        ("tone", |e| e.tone.enabled = false),
        ("color", |e| e.color.enabled = false),
        ("local", |e| e.local.clear()),
        ("detail", |e| e.detail.enabled = false),
        ("retouch", |e| e.retouch.enabled = false),
        ("geometry", |e| e.geometry.enabled = false),
        ("finishing", |e| e.finishing.enabled = false),
    ]
}

struct Stats {
    min: f64,
    median: f64,
    mean: f64,
    max: f64,
}

impl Stats {
    fn of(samples: &[f64]) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Self {
            min: sorted.first().copied().unwrap_or(0.0),
            median: sorted.get(sorted.len() / 2).copied().unwrap_or(0.0),
            mean: samples.iter().sum::<f64>() / samples.len().max(1) as f64,
            max: sorted.last().copied().unwrap_or(0.0),
        }
    }
}

fn parse_count(value: Option<&str>, default: usize, name: &str) -> Result<usize, String> {
    match value {
        None => Ok(default),
        Some(v) => match v.parse::<usize>() {
            Ok(0) | Err(_) => Err(format!("--{name} must be a positive integer, got {v:?}")),
            Ok(n) => Ok(n),
        },
    }
}

/// Parses a `WxH` size such as `2560x1707`.
fn parse_size(spec: &str) -> Result<(u32, u32), String> {
    let (w, h) = spec
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("--synthetic expects WxH, got {spec:?}"))?;
    let w = w
        .parse::<u32>()
        .map_err(|_| format!("bad width in {spec:?}"))?;
    let h = h
        .parse::<u32>()
        .map_err(|_| format!("bad height in {spec:?}"))?;
    if w == 0 || h == 0 {
        return Err(format!(
            "--synthetic dimensions must be non-zero, got {spec:?}"
        ));
    }
    Ok((w, h))
}

/// A deterministic stand-in for a decoded raw.
///
/// Content matters to the benchmark: the detail stage's noise reduction and
/// sharpening are edge-sensitive, so a flat field would under-report. This
/// generates a smooth gradient crossed by hard edges and a fine checker, with
/// no randomness, so two machines benchmark identical pixels.
fn synthetic_raw(width: u32, height: u32) -> focale_core::decode::DecodedRaw {
    let mut pixels = vec![0.0f32; width as usize * height as usize * 3];
    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            // Smooth base gradient, well inside the sensor's range.
            let base = 0.15 + 0.6 * fx * (1.0 - 0.5 * fy);
            // Hard edges every 64 px give the detail stage real gradients.
            let edge = if (x / 64 + y / 64) % 2 == 0 {
                0.08
            } else {
                -0.08
            };
            // Single-pixel checker exercises the noise-reduction kernels.
            let checker = if (x + y) % 2 == 0 { 0.015 } else { -0.015 };
            let v = (base + edge + checker).clamp(0.0, 1.0);
            let i = (y as usize * width as usize + x as usize) * 3;
            pixels[i] = v;
            pixels[i + 1] = (v * 0.94).clamp(0.0, 1.0);
            pixels[i + 2] = (v * 0.82).clamp(0.0, 1.0);
        }
    }
    focale_core::decode::DecodedRaw {
        width,
        height,
        pixels,
        metadata: focale_core::decode::RawMetadata {
            camera_make: Some("Focale".into()),
            camera_model: Some("Synthetic".into()),
            as_shot_neutral: None,
            xyz_to_camera: None,
            orientation: 1,
            capture_time: None,
            iso: None,
            exposure_time: None,
            f_number: None,
            focal_length: None,
            lens_model: None,
            optics: Default::default(),
        },
    }
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
    fn synthetic_size_parsing() {
        assert_eq!(parse_size("2560x1707").unwrap(), (2560, 1707));
        assert_eq!(parse_size("640X480").unwrap(), (640, 480));
        assert!(parse_size("2560").is_err());
        assert!(parse_size("0x480").is_err());
        assert!(parse_size("axb").is_err());
    }

    #[test]
    fn run_counts_reject_zero_and_junk() {
        assert_eq!(parse_count(None, 20, "runs").unwrap(), 20);
        assert_eq!(parse_count(Some("7"), 20, "runs").unwrap(), 7);
        assert!(parse_count(Some("0"), 20, "runs").is_err());
        assert!(parse_count(Some("many"), 20, "runs").is_err());
    }

    #[test]
    fn stats_summarize_unsorted_samples() {
        let s = Stats::of(&[30.0, 10.0, 20.0]);
        assert_eq!(s.min, 10.0);
        assert_eq!(s.max, 30.0);
        assert_eq!(s.median, 20.0);
        assert!((s.mean - 20.0).abs() < 1e-9);
    }

    #[test]
    fn synthetic_raw_is_deterministic_and_in_range() {
        let a = synthetic_raw(32, 16);
        let b = synthetic_raw(32, 16);
        assert_eq!(a.pixels, b.pixels, "same size must give identical pixels");
        assert_eq!(a.pixels.len(), 32 * 16 * 3);
        assert!(a.pixels.iter().all(|v| (0.0..=1.0).contains(v)));
        // Adjacent pixels must differ, or the detail stage has no gradients
        // to work on and the benchmark under-reports.
        assert_ne!(a.pixels[0], a.pixels[3]);
    }

    #[test]
    fn format_and_gamut_parsing() {
        assert!(format_from("tiff16").is_ok());
        assert!(format_from("bmp").is_err());
        assert!(gamut_from("rec2020").is_ok());
        assert!(gamut_from("ntsc").is_err());
    }
}
