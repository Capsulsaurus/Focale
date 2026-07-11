//! Session model: strictly one directory at a time (PRD §7).
//!
//! Opening a directory scans it (non-recursively) for raw candidates and
//! reads each sidecar's live-index block. File names and directory shape
//! carry no meaning; the sidecar is the only index source.

use std::path::{Path, PathBuf};

use focale_sidecar::SidecarDoc;
use focale_sidecar::schema::LiveIndex;

/// One raw file in the session directory.
#[derive(Debug, Clone)]
pub struct ImageEntry {
    /// Absolute path to the raw file.
    pub path: PathBuf,
    /// File name for display.
    pub file_name: String,
    /// Live-index data from the sidecar (defaults when no sidecar exists).
    pub live: LiveIndex,
}

/// A browsing session over exactly one directory.
#[derive(Debug, Default)]
pub struct Session {
    /// The open directory.
    pub dir: Option<PathBuf>,
    /// Entries in deterministic order (file name, bytewise).
    pub entries: Vec<ImageEntry>,
    /// Index of the primary (previewed) entry.
    pub primary: Option<usize>,
    /// Multi-selection (filmstrip): indices into `entries`, always
    /// containing `primary` when set. Edits broadcast to every selected
    /// entry (PRD §7 batch).
    pub selected: Vec<usize>,
}

impl Session {
    /// Scans `dir` and builds the session. Entries are sorted by file name
    /// (bytewise — deterministic regardless of filesystem order).
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let mut entries = Vec::new();
        for item in std::fs::read_dir(dir)? {
            let item = item?;
            let path = item.path();
            if !path.is_file() || !focale_core::decode::is_raw_candidate(&path) {
                continue;
            }
            let sidecar_path = focale_sidecar::sidecar_path_for(&path);
            let live = match SidecarDoc::load(&sidecar_path) {
                Ok(doc) => doc.live_index,
                Err(_) => LiveIndex::default(),
            };
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            entries.push(ImageEntry {
                path,
                file_name,
                live,
            });
        }
        entries.sort_by(|a, b| a.file_name.as_bytes().cmp(b.file_name.as_bytes()));
        let primary = if entries.is_empty() { None } else { Some(0) };
        let selected = primary.into_iter().collect();
        Ok(Self {
            dir: Some(dir.to_path_buf()),
            entries,
            primary,
            selected,
        })
    }

    /// Selects `index` as primary. `extend` keeps the existing selection
    /// (ctrl/shift-click semantics are resolved by the caller).
    pub fn select(&mut self, index: usize, extend: bool) {
        if index >= self.entries.len() {
            return;
        }
        self.primary = Some(index);
        if extend {
            if !self.selected.contains(&index) {
                self.selected.push(index);
                self.selected.sort_unstable();
            }
        } else {
            self.selected = vec![index];
        }
    }

    /// The primary entry, if any.
    pub fn primary_entry(&self) -> Option<&ImageEntry> {
        self.primary.and_then(|i| self.entries.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_scans_sorted_and_selects_first() {
        let dir = std::env::temp_dir().join(format!("focale-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.ARW"), b"x").unwrap();
        std::fs::write(dir.join("a.dng"), b"x").unwrap();
        std::fs::write(dir.join("c.txt"), b"x").unwrap();
        let s = Session::open(&dir).unwrap();
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0].file_name, "a.dng");
        assert_eq!(s.entries[1].file_name, "b.ARW");
        assert_eq!(s.primary, Some(0));
        assert_eq!(s.selected, vec![0]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn select_extend_accumulates() {
        let mut s = Session {
            dir: None,
            entries: vec![],
            primary: None,
            selected: vec![],
        };
        s.entries = (0..3)
            .map(|i| ImageEntry {
                path: PathBuf::from(format!("{i}.arw")),
                file_name: format!("{i}.arw"),
                live: LiveIndex::default(),
            })
            .collect();
        s.select(0, false);
        s.select(2, true);
        assert_eq!(s.primary, Some(2));
        assert_eq!(s.selected, vec![0, 2]);
        s.select(1, false);
        assert_eq!(s.selected, vec![1]);
    }
}
