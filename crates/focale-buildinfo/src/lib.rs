//! Build provenance for the Focale binaries.
//!
//! The single source of the strings the applications stamp into sidecar
//! debug-provenance fields (`focale_version` / `focale_platform`,
//! docs/sidecar-schema.md §5.1) and print from `--version`-style output.
//! Lives in its own leaf crate so the deterministic-path crates
//! (`focale-core`, `focale-sidecar`) stay free of build scripts.

/// The build identifier: `"<release>+<short git hash>"`, e.g.
/// `"0.1.0+e258182"`. The hash is `"unknown"` when built outside a git
/// checkout (source tarball, vendored build).
pub fn version() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("FOCALE_GIT_HASH"))
}

/// Short conventional OS name of the running platform: `"linux"`,
/// `"macos"`, or `"windows"` ([`std::env::consts::OS`], which already uses
/// these abbreviated names).
pub fn platform() -> &'static str {
    std::env::consts::OS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_release_plus_hash() {
        let v = version();
        let (release, hash) = v.split_once('+').expect("version contains '+'");
        assert_eq!(release, env!("CARGO_PKG_VERSION"));
        assert!(
            hash == "unknown" || (hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())),
            "unexpected hash segment: {hash:?}"
        );
    }

    #[test]
    fn platform_is_short_conventional_name() {
        // The supported tier; other Unixes still produce a sensible short
        // name from std, they are just not first-class targets.
        assert!(["linux", "macos", "windows"].contains(&platform()));
    }
}
