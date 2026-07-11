//! Headless Focale: export (raw + sidecar) pairs from the command line.
//!
//! This binary is the reference deterministic export path; CI runs it on
//! x86_64 and aarch64 and diffs the output bytes.

fn version_line() -> String {
    format!(
        "focale-cli {} (pipeline v{}, sidecar schema v{})",
        env!("CARGO_PKG_VERSION"),
        focale_core::PIPELINE_VERSION,
        focale_sidecar::SCHEMA_VERSION,
    )
}

fn main() {
    println!("{}", version_line());
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
}
