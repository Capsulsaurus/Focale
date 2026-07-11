//! Focale core: the deterministic raw processing pipeline.
//!
//! Everything on the export path lives here and is CPU-only, bit-identical
//! across machines and architectures. See `docs/architecture.md`.

pub mod masks;
pub mod params;

/// The current pipeline version.
///
/// Every sidecar records the pipeline version that created it, and exports
/// must reproduce old versions' output forever. Changing any algorithm's
/// output requires bumping this and keeping the old implementation.
pub const PIPELINE_VERSION: u32 = 1;

/// Returns true if `version` is a pipeline version this build can render.
pub fn supports_pipeline_version(version: u32) -> bool {
    (1..=PIPELINE_VERSION).contains(&version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_all_versions_up_to_current() {
        assert!(!supports_pipeline_version(0));
        assert!(supports_pipeline_version(PIPELINE_VERSION));
        assert!(!supports_pipeline_version(PIPELINE_VERSION + 1));
    }
}
