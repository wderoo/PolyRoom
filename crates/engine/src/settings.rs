//! The user's layout choices (thumbnail size, panels, view, open folder), saved between runs.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Collection, Module, Panels, ViewMode};

/// File name inside the catalog folder.
pub(crate) const FILE: &str = "settings.json";

/// Default thumbnail edge length in logical pixels.
pub(crate) const DEFAULT_THUMB_SIZE: f32 = 192.0;

/// Everything restored on the next start. Missing fields take their defaults, so files written
/// by older versions still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub thumb_size: f32,
    pub show_bed_grid: bool,
    pub panels: Panels,
    pub view: ViewMode,
    pub module: Module,
    pub collection: Collection,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            thumb_size: DEFAULT_THUMB_SIZE,
            show_bed_grid: true,
            panels: Panels::default(),
            view: ViewMode::default(),
            module: Module::default(),
            collection: Collection::default(),
        }
    }
}

/// Reads the settings at `path`; a missing or unreadable file gives the defaults.
pub(crate) fn load(path: &Path) -> Settings {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
        Err(e) => {
            log::warn!("could not read {}: {e}", path.display());
            return Settings::default();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("ignoring corrupt settings {}: {e}", path.display());
        Settings::default()
    })
}

/// Writes the settings atomically (temp file, then rename).
pub(crate) fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let settings = Settings {
            thumb_size: 256.0,
            show_bed_grid: false,
            panels: Panels {
                left: false,
                right: true,
                filmstrip: false,
            },
            view: ViewMode::Loupe,
            module: Module::Viewer,
            collection: Collection::Folder("C:/3D/Halloween".into()),
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_corrupt_and_partial_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(load(&path), Settings::default());

        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(load(&path), Settings::default());

        std::fs::write(&path, br#"{"thumb_size": 300.0}"#).unwrap();
        let partial = load(&path);
        assert_eq!(partial.thumb_size, 300.0);
        assert!(partial.show_bed_grid, "the grid is on unless turned off");
        assert_eq!(partial.panels, Panels::default());
        assert_eq!(partial.view, ViewMode::Grid);
    }
}
