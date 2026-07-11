//! Model discovery in the user data directory.
//!
//! Models live under `<data dir>/focale/models`, where `<data dir>` follows
//! the XDG Base Directory spec: `$XDG_DATA_HOME` when set to an absolute
//! path, otherwise `$HOME/.local/share`. The resolution is hand-rolled (a
//! dozen lines) rather than pulling in a `dirs`-style dependency.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The ONNX models this crate can load. See the crate docs for the exact
/// model provenance and licenses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// MobileSAM image encoder (image → embeddings, once per image).
    SamEncoder,
    /// SAM mask decoder (embeddings + point prompt → mask, once per click).
    SamDecoder,
    /// BiSeNet 19-class face parsing (person / person-part masks).
    FaceParsing,
    /// U²-Net salient-object model (subject / background masks).
    Saliency,
    /// U²-Net sky segmentation model.
    Sky,
}

impl ModelKind {
    /// Every model kind, in a fixed order.
    pub const ALL: [ModelKind; 5] = [
        ModelKind::SamEncoder,
        ModelKind::SamDecoder,
        ModelKind::FaceParsing,
        ModelKind::Saliency,
        ModelKind::Sky,
    ];

    /// The file name expected in the models directory. These names match
    /// what `scripts/fetch-models.sh` downloads.
    pub fn file_name(self) -> &'static str {
        match self {
            ModelKind::SamEncoder => "mobile_sam_image_encoder.onnx",
            ModelKind::SamDecoder => "sam_mask_decoder_single.onnx",
            ModelKind::FaceParsing => "face_parsing_resnet18.onnx",
            ModelKind::Saliency => "u2net.onnx",
            ModelKind::Sky => "skyseg.onnx",
        }
    }
}

/// Locates model files under a root directory.
#[derive(Debug, Clone)]
pub struct ModelPaths {
    root: PathBuf,
}

impl ModelPaths {
    /// Uses an explicit models directory (tests, portable installs).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The standard per-user location: `$XDG_DATA_HOME/focale/models` if
    /// `XDG_DATA_HOME` is set to an absolute path (per the XDG spec,
    /// relative values are ignored), else `$HOME/.local/share/focale/models`.
    pub fn user_default() -> Self {
        let data_home = user_data_home(
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        );
        Self {
            root: data_home.join("focale").join("models"),
        }
    }

    /// The models directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Full path of one model file (whether or not it exists).
    pub fn resolve(&self, kind: ModelKind) -> PathBuf {
        self.root.join(kind.file_name())
    }

    /// Whether the model file is present on disk.
    pub fn available(&self, kind: ModelKind) -> bool {
        self.resolve(kind).is_file()
    }
}

/// Pure XDG data-home resolution (testable without touching the real
/// environment): absolute `$XDG_DATA_HOME` wins, else `$HOME/.local/share`,
/// else `.local/share` relative to the current directory as a last resort.
fn user_data_home(xdg_data_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(xdg) = xdg_data_home {
        let p = Path::new(xdg);
        if p.is_absolute() {
            return p.to_path_buf();
        }
    }
    match home {
        Some(home) if !home.is_empty() => Path::new(home).join(".local").join("share"),
        _ => PathBuf::from(".local/share"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_data_home_absolute_wins() {
        let dir = user_data_home(
            Some(OsStr::new("/custom/data")),
            Some(OsStr::new("/home/u")),
        );
        assert_eq!(dir, PathBuf::from("/custom/data"));
    }

    #[test]
    fn xdg_data_home_relative_is_ignored() {
        let dir = user_data_home(Some(OsStr::new("rel/data")), Some(OsStr::new("/home/u")));
        assert_eq!(dir, PathBuf::from("/home/u/.local/share"));
    }

    #[test]
    fn falls_back_to_home_local_share() {
        let dir = user_data_home(None, Some(OsStr::new("/home/u")));
        assert_eq!(dir, PathBuf::from("/home/u/.local/share"));
    }

    #[test]
    fn resolve_appends_expected_file_names() {
        let paths = ModelPaths::new("/data/focale/models");
        assert_eq!(
            paths.resolve(ModelKind::Saliency),
            PathBuf::from("/data/focale/models/u2net.onnx")
        );
        assert_eq!(
            paths.resolve(ModelKind::SamEncoder),
            PathBuf::from("/data/focale/models/mobile_sam_image_encoder.onnx")
        );
    }

    #[test]
    fn available_is_false_for_missing_files() {
        let paths = ModelPaths::new(std::env::temp_dir().join("focale-nonexistent-models"));
        for kind in ModelKind::ALL {
            assert!(!paths.available(kind));
        }
    }
}
