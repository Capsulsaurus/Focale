//! Focale sidecar: the `.fcl` per-image edit file.
//!
//! One sidecar per image, CBOR with RFC 8949 §4.2 Core Deterministic
//! Encoding — identical edits always serialize to identical bytes.

pub mod cde;
pub mod schema;

pub use schema::{SidecarDoc, SidecarError};

/// The current sidecar schema version.
///
/// Forward-versioned with the same permanent-compatibility rule as the
/// pipeline: newer software reads every older schema forever.
pub const SCHEMA_VERSION: u32 = 1;

/// File extension for Focale sidecars, appended to the full image file name
/// (e.g. `IMG_0001.ARW.fcl`).
pub const SIDECAR_EXTENSION: &str = "fcl";

/// Returns the sidecar file name for a given image file name.
pub fn sidecar_file_name(image_file_name: &str) -> String {
    format!("{image_file_name}.{SIDECAR_EXTENSION}")
}

/// Returns the sidecar path for an image path: `.fcl` appended to the full
/// file name (`IMG_0001.ARW` → `IMG_0001.ARW.fcl`), in the same directory.
pub fn sidecar_path_for(image_path: &std::path::Path) -> std::path::PathBuf {
    let mut name = image_path.as_os_str().to_os_string();
    name.push(".");
    name.push(SIDECAR_EXTENSION);
    std::path::PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_name_appends_extension() {
        assert_eq!(sidecar_file_name("IMG_0001.ARW"), "IMG_0001.ARW.fcl");
    }

    #[test]
    fn sidecar_path_appends_to_full_name() {
        assert_eq!(
            sidecar_path_for(std::path::Path::new("/shoot/IMG_0001.ARW")),
            std::path::PathBuf::from("/shoot/IMG_0001.ARW.fcl")
        );
    }
}
