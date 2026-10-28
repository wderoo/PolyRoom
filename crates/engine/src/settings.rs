//! The user's layout choices (thumbnail size, panels, view, open folder), saved between runs.

use std::path::Path;

use serde::{Deserialize, Serialize};

use polyroom_preview::ThumbStyle;

use crate::sort::Sort;
use crate::{Collection, Panels};

/// File name inside the catalog folder.
pub(crate) const FILE: &str = "settings.json";

/// Default thumbnail edge length in logical pixels.
pub(crate) const DEFAULT_THUMB_SIZE: f32 = 192.0;

/// Thumbnail sizes offered in the settings, in pixels.
pub const THUMB_SIZES: [u32; 3] = [256, 512, 1024];

/// How thumbnails and the 3D preview look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    /// Thumbnail edge length in pixels; one of [`THUMB_SIZES`].
    pub size: u32,
    /// sRGB background of previews; `None` is transparent in thumbnails.
    pub background: Option<[u8; 3]>,
    /// sRGB colour of models in previews and the 3D view.
    pub model_color: [u8; 3],
    /// Bumped by "Regenerate" to throw away every cached thumbnail.
    pub generation: u32,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        let style = ThumbStyle::default();
        Self {
            size: style.size,
            background: style.background,
            model_color: style.model_color,
            generation: style.generation,
        }
    }
}

impl PreviewSettings {
    /// Replaces a size that is not offered.
    pub fn sanitized(self) -> Self {
        let size = if THUMB_SIZES.contains(&self.size) {
            self.size
        } else {
            Self::default().size
        };
        Self { size, ..self }
    }

    pub(crate) fn style(&self) -> ThumbStyle {
        ThumbStyle {
            size: self.sanitized().size,
            background: self.background,
            model_color: self.model_color,
            generation: self.generation,
        }
    }
}

/// Everything restored on the next start. Missing fields take their defaults, so files written
/// by older versions still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub thumb_size: f32,
    pub show_bed_grid: bool,
    pub photo_covers: bool,
    pub panels: Panels,
    pub sort: Sort,
    pub collection: Collection,
    pub preview: PreviewSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            thumb_size: DEFAULT_THUMB_SIZE,
            show_bed_grid: true,
            photo_covers: true,
            panels: Panels::default(),
            sort: Sort::default(),
            collection: Collection::default(),
            preview: PreviewSettings::default(),
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
    use crate::sort::{SortDir, SortKey};

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let settings = Settings {
            thumb_size: 256.0,
            show_bed_grid: false,
            photo_covers: false,
            panels: Panels {
                left: false,
                right: true,
            },
            sort: Sort {
                key: SortKey::Size,
                dir: SortDir::Descending,
            },
            collection: Collection::Folder("C:/3D/Halloween".into()),
            preview: PreviewSettings {
                size: 512,
                background: Some([10, 20, 30]),
                model_color: [200, 100, 50],
                generation: 4,
            },
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
        assert!(
            partial.photo_covers,
            "photo covers are on unless turned off"
        );
        assert_eq!(partial.panels, Panels::default());
        assert_eq!(partial.sort, Sort::default());
        assert_eq!(partial.preview, PreviewSettings::default());
    }

    #[test]
    fn settings_from_before_the_filmstrip_and_viewer_went_away_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(
            &path,
            br#"{"thumb_size": 250.0, "view": "Loupe", "module": "Viewer",
                 "panels": {"left": false, "right": true, "filmstrip": true}}"#,
        )
        .unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.thumb_size, 250.0);
        assert!(!loaded.panels.left && loaded.panels.right);
    }

    #[test]
    fn unoffered_preview_sizes_fall_back() {
        let odd = PreviewSettings {
            size: 300,
            ..PreviewSettings::default()
        };
        assert_eq!(odd.sanitized().size, 256);
        assert_eq!(odd.style().size, 256);
        let big = PreviewSettings {
            size: 1024,
            ..PreviewSettings::default()
        };
        assert_eq!(big.style().size, 1024);
    }
}
